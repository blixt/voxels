use candle_core::{Result, Tensor};

const SIGMA_MIN: f64 = 0.002;
const SIGMA_MAX: f64 = 80.0;
const SIGMA_DATA: f64 = 0.5;
const RHO: f64 = 7.0;

#[derive(Clone, Debug)]
pub struct DpmSolver {
    sigmas: Vec<f64>,
    previous_denoised: Option<Tensor>,
}

impl DpmSolver {
    pub fn new(steps: usize) -> Self {
        let maximum = SIGMA_MAX.powf(1.0 / RHO);
        let minimum = SIGMA_MIN.powf(1.0 / RHO);
        let mut sigmas = (0..steps)
            .map(|index| {
                let ramp = index as f64 / (steps - 1) as f64;
                (maximum + ramp * (minimum - maximum)).powf(RHO)
            })
            .collect::<Vec<_>>();
        sigmas.push(0.0);
        Self {
            sigmas,
            previous_denoised: None,
        }
    }

    pub fn sigmas(&self) -> &[f64] {
        &self.sigmas
    }

    pub fn scaled_input(&self, sample: &Tensor, step: usize) -> Result<Tensor> {
        let sigma = self.sigmas[step];
        sample.affine(1.0 / (sigma * sigma + SIGMA_DATA * SIGMA_DATA).sqrt(), 0.0)
    }

    pub fn noise_label(&self, step: usize) -> f32 {
        (self.sigmas[step] / SIGMA_DATA).atan() as f32
    }

    pub fn step(&mut self, model_output: &Tensor, sample: &Tensor, step: usize) -> Result<Tensor> {
        let sigma = self.sigmas[step];
        let next_sigma = self.sigmas[step + 1];
        let denominator = sigma * sigma + SIGMA_DATA * SIGMA_DATA;
        let skip = SIGMA_DATA * SIGMA_DATA / denominator;
        let output = sigma * SIGMA_DATA / denominator.sqrt();
        let denoised = (sample.affine(skip, 0.0)? + model_output.affine(output, 0.0)?)?;
        let ratio = next_sigma / sigma;
        let result = if let Some(previous) = &self.previous_denoised
            && step + 1 < self.sigmas.len() - 1
        {
            let previous_sigma = self.sigmas[step - 1];
            let h = (sigma / next_sigma).ln();
            let previous_h = (previous_sigma / sigma).ln();
            let reciprocal_r = h / previous_h;
            let derivative = (denoised.clone() - previous)?.affine(reciprocal_r, 0.0)?;
            ((sample.affine(ratio, 0.0)? + denoised.affine(1.0 - ratio, 0.0)?)?
                + derivative.affine(0.5 * (1.0 - ratio), 0.0)?)?
        } else {
            (sample.affine(ratio, 0.0)? + denoised.affine(1.0 - ratio, 0.0)?)?
        };
        self.previous_denoised = Some(denoised);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_step_schedule_matches_published_values() {
        let scheduler = DpmSolver::new(20);
        let expected = [
            80.0,
            59.657_526,
            43.920_260_2,
            31.884_428_6,
            22.794_110_6,
            16.022_301_6,
            11.053_666_8,
            7.468_906_98,
            4.930_657_81,
            3.170_842_74,
            1.979_400_65,
            1.194_309_07,
            0.692_823_757,
            0.383_855_363,
            0.201_404_134,
            0.098_973_383_2,
            0.044_882_572_4,
            0.018_400_829_9,
            0.006_621_706_89,
            0.002,
            0.0,
        ];
        for (&actual, expected) in scheduler.sigmas().iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }
}
