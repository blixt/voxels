use candle_core::{DType, Device, Result, Tensor};
use candle_nn::{VarBuilder, ops};
use serde::Deserialize;
use std::path::Path;

const NORMALIZE_EPSILON: f64 = 1.0e-4;
const MP_SILU_DIVISOR: f64 = 0.596;
const CHANNELS_PER_HEAD: usize = 64;

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum LayerCount {
    One(usize),
    Each(Vec<usize>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum FourierScale {
    Name(String),
    Number(f64),
}

#[derive(Clone, Debug, Deserialize)]
pub struct EdmUnetConfig {
    pub image_size: usize,
    pub in_channels: usize,
    pub out_channels: Option<usize>,
    pub model_channels: usize,
    pub model_channel_mults: Option<Vec<usize>>,
    layers_per_block: LayerCount,
    pub emb_channels: Option<usize>,
    pub noise_emb_dims: Option<usize>,
    pub attn_resolutions: Option<Vec<usize>>,
    pub midblock_attention: bool,
    pub concat_balance: f64,
    pub conditional_inputs: Vec<(String, usize, f64)>,
    pub encode_only: bool,
    pub disable_out_gain: bool,
    fourier_scale: FourierScale,
}

impl EdmUnetConfig {
    pub fn from_path(path: &Path) -> std::result::Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    fn block_counts(&self, levels: usize) -> Result<Vec<usize>> {
        match &self.layers_per_block {
            LayerCount::One(count) => Ok(vec![*count; levels]),
            LayerCount::Each(counts) if counts.len() == levels => Ok(counts.clone()),
            LayerCount::Each(counts) => candle_core::bail!(
                "layers_per_block has {} entries for {levels} levels",
                counts.len()
            ),
        }
    }

    fn uses_positional_noise(&self) -> Result<bool> {
        match &self.fourier_scale {
            FourierScale::Name(name) if name == "pos" => Ok(true),
            FourierScale::Name(name) => candle_core::bail!("unsupported Fourier scale {name}"),
            FourierScale::Number(scale) => {
                let _ = scale;
                Ok(false)
            }
        }
    }
}

#[derive(Clone, Debug)]
struct MpConv {
    weight: Tensor,
    padding: usize,
    groups: usize,
}

impl MpConv {
    fn load(
        builder: VarBuilder<'_>,
        in_channels: usize,
        out_channels: usize,
        kernel: Option<usize>,
        groups: usize,
    ) -> Result<Self> {
        let target_dtype = builder.dtype();
        let builder = builder.to_dtype(DType::F32);
        let weight = match kernel {
            Some(kernel) => builder.get(
                (out_channels, in_channels / groups, kernel, kernel),
                "weight",
            )?,
            None => builder.get((out_channels, in_channels), "weight")?,
        };
        let per_output = weight.elem_count() / out_channels;
        // Upstream `normalize` is RMS normalization: eps + L2/sqrt(element_count).
        // Keep this in F32 before copying the prepared weight to the requested device dtype.
        let denominator = weight
            .sqr()?
            .sum_all()?
            .sqrt()?
            .affine(1.0 / (weight.elem_count() as f64).sqrt(), NORMALIZE_EPSILON)?;
        let weight = weight
            .broadcast_div(&denominator)?
            .affine(1.0 / (per_output as f64).sqrt(), 0.0)?
            .to_dtype(target_dtype)?;
        Ok(Self {
            weight,
            padding: kernel.map_or(0, |kernel| kernel / 2),
            groups,
        })
    }

    fn forward(&self, input: &Tensor, gain: Option<&Tensor>) -> Result<Tensor> {
        let output = match self.weight.rank() {
            2 => input.matmul(&self.weight.t()?)?,
            4 => input.conv2d(&self.weight, self.padding, 1, 1, self.groups)?,
            rank => candle_core::bail!("MP convolution has unsupported rank {rank}"),
        };
        match gain {
            Some(gain) => output.broadcast_mul(gain),
            None => Ok(output),
        }
    }
}

#[derive(Clone, Debug)]
enum ScalarEmbedding {
    Positional { frequencies: Tensor },
    Fourier { frequencies: Tensor, phases: Tensor },
}

impl ScalarEmbedding {
    fn positional(builder: VarBuilder<'_>, channels: usize) -> Result<Self> {
        Ok(Self::Positional {
            frequencies: builder.get(channels / 2, "freqs")?,
        })
    }

    fn fourier(builder: VarBuilder<'_>, channels: usize) -> Result<Self> {
        Ok(Self::Fourier {
            frequencies: builder.get(channels, "freqs")?,
            phases: builder.get(channels, "phases")?,
        })
    }

    fn forward(&self, input: &Tensor) -> Result<Tensor> {
        let original_dtype = input.dtype();
        let input = input.to_dtype(DType::F32)?.unsqueeze(1)?;
        match self {
            Self::Positional { frequencies } => {
                let angles =
                    input.broadcast_mul(&frequencies.to_dtype(DType::F32)?.unsqueeze(0)?)?;
                Tensor::cat(&[angles.sin()?, angles.cos()?], 1)?
                    .affine(std::f64::consts::SQRT_2, 0.0)?
                    .to_dtype(original_dtype)
            }
            Self::Fourier {
                frequencies,
                phases,
            } => input
                .broadcast_mul(&frequencies.to_dtype(DType::F32)?.unsqueeze(0)?)?
                .broadcast_add(&phases.to_dtype(DType::F32)?.unsqueeze(0)?)?
                .cos()?
                .affine(std::f64::consts::SQRT_2, 0.0)?
                .to_dtype(original_dtype),
        }
    }
}

#[derive(Clone, Debug)]
enum ConditionalLayer {
    Float {
        embedding: ScalarEmbedding,
        projection: MpConv,
    },
    Tensor(MpConv),
}

impl ConditionalLayer {
    fn forward(&self, input: &Tensor) -> Result<Tensor> {
        match self {
            Self::Float {
                embedding,
                projection,
            } => projection.forward(&embedding.forward(input)?, None),
            Self::Tensor(projection) => mp_silu(&projection.forward(input, None)?),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockMode {
    Encoder,
    Decoder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResampleMode {
    Keep,
    Down,
    Up,
}

#[derive(Clone, Debug)]
struct UnetBlock {
    mode: BlockMode,
    resample: ResampleMode,
    out_channels: usize,
    residual_balance: f64,
    attention_balance: f64,
    embedding_gain: Tensor,
    residual_in: MpConv,
    embedding: Option<MpConv>,
    residual_out: MpConv,
    skip: Option<MpConv>,
    attention_qkv: Option<MpConv>,
    attention_projection: Option<MpConv>,
}

impl UnetBlock {
    #[allow(
        clippy::too_many_arguments,
        reason = "the parameters mirror one published U-Net block"
    )]
    fn load(
        builder: VarBuilder<'_>,
        in_channels: usize,
        out_channels: usize,
        embedding_channels: usize,
        mode: BlockMode,
        resample: ResampleMode,
        attention: bool,
    ) -> Result<Self> {
        let residual_in_channels = if mode == BlockMode::Encoder {
            out_channels
        } else {
            in_channels
        };
        let residual_in = MpConv::load(
            builder.pp("conv_res0"),
            residual_in_channels,
            out_channels,
            Some(3),
            1,
        )?;
        let embedding = (embedding_channels > 0)
            .then(|| {
                MpConv::load(
                    builder.pp("emb_linear"),
                    embedding_channels,
                    out_channels,
                    None,
                    1,
                )
            })
            .transpose()?;
        let residual_out = MpConv::load(
            builder.pp("conv_res1"),
            out_channels,
            out_channels,
            Some(3),
            1,
        )?;
        let skip = (in_channels != out_channels)
            .then(|| {
                MpConv::load(
                    builder.pp("conv_skip"),
                    in_channels,
                    out_channels,
                    Some(1),
                    1,
                )
            })
            .transpose()?;
        let attention_qkv = attention
            .then(|| {
                MpConv::load(
                    builder.pp("attn_qkv"),
                    out_channels,
                    out_channels * 3,
                    Some(1),
                    1,
                )
            })
            .transpose()?;
        let attention_projection = attention
            .then(|| {
                MpConv::load(
                    builder.pp("attn_proj"),
                    out_channels,
                    out_channels,
                    Some(1),
                    1,
                )
            })
            .transpose()?;
        Ok(Self {
            mode,
            resample,
            out_channels,
            residual_balance: 0.3,
            attention_balance: 0.3,
            embedding_gain: builder.get((), "emb_gain")?,
            residual_in,
            embedding,
            residual_out,
            skip,
            attention_qkv,
            attention_projection,
        })
    }

    fn forward(&self, input: &Tensor, embedding: Option<&Tensor>) -> Result<Tensor> {
        let mut input = match self.resample {
            ResampleMode::Keep => input.clone(),
            ResampleMode::Down => input.avg_pool2d_with_stride(1, 2)?,
            ResampleMode::Up => {
                let (_, _, height, width) = input.dims4()?;
                input.upsample_nearest2d(height * 2, width * 2)?
            }
        };
        if self.mode == BlockMode::Encoder {
            if let Some(skip) = &self.skip {
                input = skip.forward(&input, None)?;
            }
            input = normalize_dimension(&input, 1, self.out_channels)?;
        }

        let mut residual = self.residual_in.forward(&mp_silu(&input)?, None)?;
        if let (Some(projection), Some(embedding)) = (&self.embedding, embedding) {
            let conditioning = projection
                .forward(embedding, Some(&self.embedding_gain))?
                .affine(1.0, 1.0)?;
            let channels = conditioning.dim(1)?;
            let denominator = (conditioning.sqr()?.sum_keepdim(1)? / channels as f64)?
                .affine(1.0, 1.0e-8)?
                .sqrt()?;
            let conditioning = conditioning
                .broadcast_div(&denominator)?
                .unsqueeze(2)?
                .unsqueeze(3)?;
            residual = mp_silu(&residual.broadcast_mul(&conditioning)?)?;
        } else {
            residual = mp_silu(&residual)?;
        }
        residual = self.residual_out.forward(&residual, None)?;

        if self.mode == BlockMode::Decoder
            && let Some(skip) = &self.skip
        {
            input = skip.forward(&input, None)?;
        }
        let mut output = mp_sum(
            &[input, residual],
            &[1.0 - self.residual_balance, self.residual_balance],
        )?;
        if self.attention_qkv.is_some() {
            let attention = self.attention(&output)?;
            output = mp_sum(
                &[output, attention],
                &[1.0 - self.attention_balance, self.attention_balance],
            )?;
        }
        output.clamp(-256.0, 256.0)
    }

    fn attention(&self, input: &Tensor) -> Result<Tensor> {
        let qkv = self
            .attention_qkv
            .as_ref()
            .ok_or_else(|| candle_core::Error::Msg("attention weights are missing".to_owned()))?
            .forward(input, None)?;
        let (batch, _, height, width) = qkv.dims4()?;
        let heads = self.out_channels / CHANNELS_PER_HEAD;
        let spatial = height * width;
        let qkv = qkv.reshape((batch, heads, CHANNELS_PER_HEAD, 3, spatial))?;
        let parts = qkv.chunk(3, 3)?;
        let query = normalize_dimension(&parts[0].squeeze(3)?, 2, CHANNELS_PER_HEAD)?;
        let key = normalize_dimension(&parts[1].squeeze(3)?, 2, CHANNELS_PER_HEAD)?;
        let value = normalize_dimension(&parts[2].squeeze(3)?, 2, CHANNELS_PER_HEAD)?;
        let attended = ops::sdpa(
            &query.transpose(2, 3)?.contiguous()?,
            &key.transpose(2, 3)?.contiguous()?,
            &value.transpose(2, 3)?.contiguous()?,
            None,
            false,
            1.0 / (CHANNELS_PER_HEAD as f32).sqrt(),
            1.0,
        )?
        .transpose(2, 3)?
        .contiguous()?
        .reshape((batch, self.out_channels, height, width))?;
        self.attention_projection
            .as_ref()
            .ok_or_else(|| candle_core::Error::Msg("attention projection is missing".to_owned()))?
            .forward(&attended, None)
    }
}

#[derive(Clone, Debug)]
enum EncoderLayer {
    Conv(MpConv),
    Block(UnetBlock),
}

#[derive(Clone, Debug)]
struct DecoderLayer {
    concatenate_skip: bool,
    block: UnetBlock,
}

#[derive(Clone, Debug)]
pub struct EdmUnet {
    config: EdmUnetConfig,
    noise_embedding: ScalarEmbedding,
    noise_projection: MpConv,
    conditional_layers: Vec<ConditionalLayer>,
    conditional_weights: Vec<f64>,
    output_gain: Option<Tensor>,
    encoder: Vec<EncoderLayer>,
    decoder: Vec<DecoderLayer>,
    output: MpConv,
}

impl EdmUnet {
    pub fn load(
        config: EdmUnetConfig,
        weights: &Path,
        dtype: DType,
        device: &Device,
    ) -> Result<Self> {
        // SAFETY: Candle keeps the memory maps alive in the returned VarBuilder backend, the file is
        // immutable for the lifetime of model construction, and every requested tensor is copied to
        // the Metal device before this function returns.
        let builder = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], dtype, device)? };
        let channel_multipliers = config
            .model_channel_mults
            .clone()
            .unwrap_or_else(|| vec![1, 2, 3, 4]);
        let block_counts = config.block_counts(channel_multipliers.len())?;
        let block_channels = channel_multipliers
            .iter()
            .map(|multiplier| config.model_channels * multiplier)
            .collect::<Vec<_>>();
        let embedding_channels = config.emb_channels.unwrap_or_else(|| {
            block_channels
                .iter()
                .copied()
                .max()
                .unwrap_or(config.model_channels)
        });
        let noise_embedding_channels = config.noise_emb_dims.unwrap_or(config.model_channels);
        if !config.uses_positional_noise()? {
            candle_core::bail!(
                "published Terrain Diffusion models require positional noise embeddings"
            );
        }
        let noise_embedding =
            ScalarEmbedding::positional(builder.pp("noise_fourier"), noise_embedding_channels)?;
        let noise_projection = MpConv::load(
            builder.pp("noise_linear"),
            noise_embedding_channels,
            embedding_channels,
            None,
            1,
        )?;

        let mut conditional_layers = Vec::with_capacity(config.conditional_inputs.len());
        let mut conditional_weights = vec![1.0];
        for (index, (kind, dimensions, weight)) in config.conditional_inputs.iter().enumerate() {
            let prefix = builder.pp(format!("conditional_layers.{index}"));
            let layer = match kind.as_str() {
                "float" => ConditionalLayer::Float {
                    embedding: ScalarEmbedding::fourier(prefix.pp("0"), *dimensions)?,
                    projection: MpConv::load(
                        prefix.pp("1"),
                        *dimensions,
                        embedding_channels,
                        None,
                        1,
                    )?,
                },
                "tensor" => ConditionalLayer::Tensor(MpConv::load(
                    prefix,
                    *dimensions,
                    embedding_channels,
                    None,
                    1,
                )?),
                other => candle_core::bail!("unsupported conditional input {other}"),
            };
            conditional_layers.push(layer);
            conditional_weights.push(*weight);
        }

        let mut encoder = Vec::new();
        let mut output_channels = config.in_channels + 1;
        let attention_resolutions = config.attn_resolutions.clone().unwrap_or_default();
        for (level, (&channels, &blocks)) in block_channels.iter().zip(&block_counts).enumerate() {
            let resolution = config.image_size / 2usize.pow(level as u32);
            if level == 0 {
                encoder.push(EncoderLayer::Conv(MpConv::load(
                    builder.pp(format!("enc.{resolution}x{resolution}_conv")),
                    output_channels,
                    channels,
                    Some(3),
                    1,
                )?));
                output_channels = channels;
            } else {
                encoder.push(EncoderLayer::Block(UnetBlock::load(
                    builder.pp(format!("enc.{resolution}x{resolution}_down")),
                    output_channels,
                    output_channels,
                    embedding_channels,
                    BlockMode::Encoder,
                    ResampleMode::Down,
                    false,
                )?));
            }
            for index in 0..blocks {
                let input_channels = output_channels;
                output_channels = channels;
                encoder.push(EncoderLayer::Block(UnetBlock::load(
                    builder.pp(format!("enc.{resolution}x{resolution}_block{index}")),
                    input_channels,
                    output_channels,
                    embedding_channels,
                    BlockMode::Encoder,
                    ResampleMode::Keep,
                    attention_resolutions.contains(&resolution),
                )?));
            }
        }

        let mut decoder = Vec::new();
        if !config.encode_only {
            let mut skip_channels = encoder
                .iter()
                .map(|layer| match layer {
                    EncoderLayer::Conv(conv) => conv.weight.dim(0),
                    EncoderLayer::Block(block) => Ok(block.out_channels),
                })
                .collect::<Result<Vec<_>>>()?;
            for level in (0..block_channels.len()).rev() {
                let channels = block_channels[level];
                let blocks = block_counts[level];
                let resolution = config.image_size / 2usize.pow(level as u32);
                if level == block_channels.len() - 1 {
                    for (name, attention) in [("in0", config.midblock_attention), ("in1", false)] {
                        decoder.push(DecoderLayer {
                            concatenate_skip: false,
                            block: UnetBlock::load(
                                builder.pp(format!("dec.{resolution}x{resolution}_{name}")),
                                output_channels,
                                output_channels,
                                embedding_channels,
                                BlockMode::Decoder,
                                ResampleMode::Keep,
                                attention,
                            )?,
                        });
                    }
                } else {
                    decoder.push(DecoderLayer {
                        concatenate_skip: false,
                        block: UnetBlock::load(
                            builder.pp(format!("dec.{resolution}x{resolution}_up")),
                            output_channels,
                            output_channels,
                            embedding_channels,
                            BlockMode::Decoder,
                            ResampleMode::Up,
                            false,
                        )?,
                    });
                }
                for index in 0..=blocks {
                    let skip_channels = skip_channels.pop().ok_or_else(|| {
                        candle_core::Error::Msg("decoder requested a missing skip".to_owned())
                    })?;
                    let input_channels = output_channels + skip_channels;
                    output_channels = channels;
                    decoder.push(DecoderLayer {
                        concatenate_skip: true,
                        block: UnetBlock::load(
                            builder.pp(format!("dec.{resolution}x{resolution}_block{index}")),
                            input_channels,
                            output_channels,
                            embedding_channels,
                            BlockMode::Decoder,
                            ResampleMode::Keep,
                            attention_resolutions.contains(&resolution),
                        )?,
                    });
                }
            }
        }
        let output = MpConv::load(
            builder.pp("out_conv"),
            output_channels,
            config.out_channels.unwrap_or(config.in_channels),
            Some(3),
            1,
        )?;
        let output_gain = (!config.disable_out_gain)
            .then(|| builder.get((), "out_gain"))
            .transpose()?;
        Ok(Self {
            config,
            noise_embedding,
            noise_projection,
            conditional_layers,
            conditional_weights,
            output_gain,
            encoder,
            decoder,
            output,
        })
    }

    pub fn forward(
        &self,
        input: &Tensor,
        noise_labels: &Tensor,
        conditional_inputs: &[Tensor],
    ) -> Result<Tensor> {
        if conditional_inputs.len() != self.conditional_layers.len() {
            candle_core::bail!(
                "expected {} conditional inputs, got {}",
                self.conditional_layers.len(),
                conditional_inputs.len()
            );
        }
        let mut embeddings = vec![
            self.noise_projection
                .forward(&self.noise_embedding.forward(noise_labels)?, None)?,
        ];
        for (layer, input) in self.conditional_layers.iter().zip(conditional_inputs) {
            embeddings.push(layer.forward(input)?);
        }
        let embedding = mp_silu(&mp_sum(&embeddings, &self.conditional_weights)?)?;
        let (batch, _, height, width) = input.dims4()?;
        let ones = Tensor::ones((batch, 1, height, width), input.dtype(), input.device())?;
        let mut value = Tensor::cat(&[input, &ones], 1)?;
        let mut skips = Vec::with_capacity(self.encoder.len());
        for layer in &self.encoder {
            value = match layer {
                EncoderLayer::Conv(conv) => conv.forward(&value, None)?,
                EncoderLayer::Block(block) => block.forward(&value, Some(&embedding))?,
            };
            skips.push(value.clone());
        }
        for layer in &self.decoder {
            if layer.concatenate_skip {
                let skip = skips.pop().ok_or_else(|| {
                    candle_core::Error::Msg("decoder requested a missing skip tensor".to_owned())
                })?;
                value = mp_concat(&[value, skip], self.config.concat_balance)?;
            }
            value = layer.block.forward(&value, Some(&embedding))?;
        }
        self.output.forward(&value, self.output_gain.as_ref())
    }
}

fn normalize_dimension(input: &Tensor, dimension: usize, length: usize) -> Result<Tensor> {
    let denominator = input
        .sqr()?
        .sum_keepdim(dimension)?
        .sqrt()?
        .affine(1.0 / (length as f64).sqrt(), NORMALIZE_EPSILON)?;
    input.broadcast_div(&denominator)
}

fn mp_silu(input: &Tensor) -> Result<Tensor> {
    input.silu()?.affine(1.0 / MP_SILU_DIVISOR, 0.0)
}

fn mp_sum(inputs: &[Tensor], weights: &[f64]) -> Result<Tensor> {
    if inputs.is_empty() || inputs.len() != weights.len() {
        candle_core::bail!("magnitude-preserving sum has mismatched inputs and weights");
    }
    let norm = weights
        .iter()
        .map(|weight| weight * weight)
        .sum::<f64>()
        .sqrt();
    let mut output = inputs[0].affine(weights[0], 0.0)?;
    for (input, weight) in inputs.iter().zip(weights).skip(1) {
        output = (output + input.affine(*weight, 0.0)?)?;
    }
    output.affine(1.0 / norm, 0.0)
}

fn mp_concat(inputs: &[Tensor; 2], second_weight: f64) -> Result<Tensor> {
    let weights = [1.0 - second_weight, second_weight];
    let channels = [inputs[0].dim(1)?, inputs[1].dim(1)?];
    let total_channels = channels[0] + channels[1];
    let weight_norm_squared = weights[0] * weights[0] + weights[1] * weights[1];
    let common = (total_channels as f64 / weight_norm_squared).sqrt();
    let first = inputs[0].affine(common / (channels[0] as f64).sqrt() * weights[0], 0.0)?;
    let second = inputs[1].affine(common / (channels[1] as f64).sqrt() * weights[1], 0.0)?;
    Tensor::cat(&[first, second], 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_configs_parse_without_losing_model_shape() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        for (name, expected_in, expected_out) in [
            ("coarse-config.json", 11, 6),
            ("base-config.json", 5, 5),
            ("decoder-config.json", 5, 1),
        ] {
            let config = EdmUnetConfig::from_path(&root.join(name)).expect("fixture parses");
            assert_eq!(config.in_channels, expected_in);
            assert_eq!(config.out_channels, Some(expected_out));
            assert!(config.uses_positional_noise().expect("known embedding"));
        }
    }

    #[test]
    fn dimension_normalization_preserves_unit_rms() {
        let input = Tensor::ones((1, 4), DType::F32, &Device::Cpu).expect("input");
        let output = normalize_dimension(&input, 1, 4)
            .and_then(|tensor| tensor.flatten_all())
            .and_then(|tensor| tensor.to_vec1::<f32>())
            .expect("normalized values");
        for value in output {
            assert!((value - 0.999_900_04).abs() < 1.0e-6);
        }
    }
}
