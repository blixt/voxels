//! Versioned, host-testable client configuration.
//!
//! This crate owns only settings used by the game client. World identity, world-source selection,
//! gameplay rules, and authored debug destinations belong to the world service and deliberately do
//! not appear here. File I/O also stays in the host shell: it reads TOML and passes the contents to
//! [`ClientConfig::from_toml`].

use serde::{Deserialize, Serialize};
use std::fmt;

pub const CLIENT_CONFIG_SCHEMA_VERSION: u32 = 16;

const MAX_FIXED_STEP_SECONDS: f32 = 0.1;
const MAX_SIMULATION_STEPS_PER_FRAME: u32 = 64;
const MAX_EDIT_TRACKERS: u32 = 65_536;
const MAX_SIGNED_RUNTIME_INTEGER: u32 = i32::MAX as u32;
const MAX_TRACKED_CHUNKS: u32 = 1_048_576;
const MAX_SURFACE_RADIUS_TILES: u32 = 64;
const MAX_FRAME_STAGE_BUDGET: u32 = 65_536;
const MAX_VIEW_DISTANCE_METRES: f32 = 100_000.0;
const MAX_SHADOW_MAP_RESOLUTION: u32 = 4_096;
const MAX_DIAGNOSTIC_INTERVAL_MS: u32 = 3_600_000;
const MAX_WORLD_IN_FLIGHT_BATCHES: u32 = 256;
const MAX_WORLD_BUFFERED_BYTES: u32 = 64 * 1024 * 1024;
const MAX_WORLD_REQUEST_TIMEOUT_MS: u32 = 300_000;
const MAX_WORLD_RECONNECT_DELAY_MS: u32 = 60_000;
const MAX_WORLD_RECONNECT_ATTEMPTS: u32 = 10_000;
const MAX_PRESENCE_BUFFERED_BYTES: u32 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub schema_version: u32,
    pub developer: DeveloperConfig,
    pub world: WorldTransportConfig,
    pub multiplayer: MultiplayerConfig,
    pub runtime: RuntimeConfig,
    pub streaming: StreamingConfig,
    pub rendering: RenderingConfig,
    pub diagnostics: DiagnosticsConfig,
    pub profiling: ProfilingConfig,
}

/// Client-side developer affordances. Server-advertised capabilities remain the authority for
/// actions such as creative flight; this flag only decides whether the local UI may request them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeveloperConfig {
    pub controls_enabled: bool,
}

/// How the client reaches the authoritative world service. Provider selection deliberately is not
/// represented here; procedural and learned terrain use the same negotiated product protocol.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldTransportConfig {
    pub endpoint: String,
    pub presence_endpoint: String,
    pub subprotocol: String,
    /// Second WebSocket protocol token used as loopback authorization. It must match the native
    /// service's `transport.auth_subprotocol_token` and is never selected as the wire protocol.
    pub auth_subprotocol_token: String,
    pub max_in_flight_batches: u32,
    pub buffered_amount_high_water_bytes: u32,
    pub buffered_amount_low_water_bytes: u32,
    pub request_timeout_ms: u32,
    pub reconnect_initial_delay_ms: u32,
    pub reconnect_max_delay_ms: u32,
    pub reconnect_attempt_limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiplayerConfig {
    pub pose_send_interval_ms: u32,
    pub clock_sync_interval_ms: u32,
    pub buffered_amount_high_water_bytes: u32,
    pub interpolation_delay_ms: u32,
    pub min_interpolation_delay_ms: u32,
    pub max_interpolation_delay_ms: u32,
    pub max_extrapolation_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub fixed_step_seconds: f32,
    pub max_steps_per_frame: u32,
    pub max_edit_trackers: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamingConfig {
    pub load_radius_chunks: u32,
    pub vertical_radius_chunks: u32,
    /// Horizontal radius that must be fully resident before gameplay input is enabled.
    pub startup_ready_radius_chunks: u32,
    pub retention_margin_chunks: u32,
    pub max_tracked_chunks: u32,
    pub max_secondary_interest_chunks: u32,
    pub frame_budget: FrameBudgetConfig,
    pub surface: SurfaceStreamingConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameBudgetConfig {
    pub generation: u32,
    pub meshing: u32,
    pub upload: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStreamingConfig {
    pub load_radius_tiles: [u32; 6],
    pub retention_margin_tiles: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderingConfig {
    pub view_distance_metres: f32,
    pub shadows: ShadowConfig,
    pub volumetric_clouds: VolumetricCloudConfig,
    pub features: RendererFeatureConfig,
    pub mission_control: MissionControlConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumetricCloudConfig {
    pub enabled: bool,
    pub resolution_scale: f32,
    pub view_steps: u32,
    pub light_steps: u32,
    pub max_distance_metres: f32,
    pub extinction: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowConfig {
    pub vertical_fov_radians: f32,
    pub near_plane: f32,
    pub far_plane: f32,
    pub split_lambda: f32,
    pub shadow_map_resolution: u32,
    pub direction_quantization_radians: f32,
    pub caster_depth_expansion: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererFeatureConfig {
    pub cascaded_sun_shadows: bool,
    pub voxel_ambient_occlusion: bool,
    pub screen_space_ambient_occlusion: bool,
    pub atmospheric_fog: bool,
    pub far_terrain: bool,
    pub water_surface: bool,
    pub target_outline: bool,
    pub material_surface_detail: bool,
    pub cave_headlamp: bool,
    pub voxel_emissive_lights: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionControlConfig {
    pub open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsConfig {
    pub enclosure_probe_interval_ms: u32,
    pub enclosure_probe_distance_metres: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilingConfig {
    pub speed_metres_per_second: f32,
    pub warmup_seconds: f32,
    pub measure_seconds: f32,
}

impl ClientConfig {
    pub fn from_toml(contents: &str) -> Result<Self, ClientConfigError> {
        let config: Self = toml::from_str(contents)
            .map_err(|error| ClientConfigError::Parse(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, ClientConfigError> {
        self.validate()?;
        toml::to_string_pretty(self)
            .map_err(|error| ClientConfigError::Serialize(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), ClientConfigError> {
        if self.schema_version != CLIENT_CONFIG_SCHEMA_VERSION {
            return Err(ClientConfigError::UnsupportedSchema {
                expected: CLIENT_CONFIG_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }

        self.world.validate()?;
        self.multiplayer.validate()?;

        ensure_finite_range(
            self.runtime.fixed_step_seconds,
            "runtime.fixed_step_seconds",
            0.0,
            MAX_FIXED_STEP_SECONDS,
            false,
        )?;
        ensure_integer_range(
            self.runtime.max_steps_per_frame,
            "runtime.max_steps_per_frame",
            1,
            MAX_SIMULATION_STEPS_PER_FRAME,
        )?;
        ensure_integer_range(
            self.runtime.max_edit_trackers,
            "runtime.max_edit_trackers",
            1,
            MAX_EDIT_TRACKERS,
        )?;

        ensure_integer_range(
            self.streaming.load_radius_chunks,
            "streaming.load_radius_chunks",
            0,
            MAX_SIGNED_RUNTIME_INTEGER,
        )?;
        ensure_integer_range(
            self.streaming.vertical_radius_chunks,
            "streaming.vertical_radius_chunks",
            0,
            MAX_SIGNED_RUNTIME_INTEGER,
        )?;
        ensure_integer_range(
            self.streaming.startup_ready_radius_chunks,
            "streaming.startup_ready_radius_chunks",
            0,
            self.streaming.load_radius_chunks,
        )?;
        ensure_integer_range(
            self.streaming.retention_margin_chunks,
            "streaming.retention_margin_chunks",
            0,
            MAX_SIGNED_RUNTIME_INTEGER,
        )?;
        ensure_integer_range(
            self.streaming.max_tracked_chunks,
            "streaming.max_tracked_chunks",
            1,
            MAX_TRACKED_CHUNKS,
        )?;
        ensure_integer_range(
            self.streaming.max_secondary_interest_chunks,
            "streaming.max_secondary_interest_chunks",
            0,
            self.streaming.max_tracked_chunks,
        )?;
        for (field, value) in [
            (
                "streaming.frame_budget.generation",
                self.streaming.frame_budget.generation,
            ),
            (
                "streaming.frame_budget.meshing",
                self.streaming.frame_budget.meshing,
            ),
            (
                "streaming.frame_budget.upload",
                self.streaming.frame_budget.upload,
            ),
        ] {
            ensure_integer_range(value, field, 1, MAX_FRAME_STAGE_BUDGET)?;
        }
        for radius in self.streaming.surface.load_radius_tiles {
            ensure_integer_range(
                radius,
                "streaming.surface.load_radius_tiles",
                0,
                MAX_SURFACE_RADIUS_TILES,
            )?;
        }
        ensure_integer_range(
            self.streaming.surface.retention_margin_tiles,
            "streaming.surface.retention_margin_tiles",
            0,
            MAX_SURFACE_RADIUS_TILES,
        )?;

        ensure_finite_range(
            self.rendering.view_distance_metres,
            "rendering.view_distance_metres",
            0.0,
            MAX_VIEW_DISTANCE_METRES,
            false,
        )?;
        let shadows = self.rendering.shadows;
        if !shadows.vertical_fov_radians.is_finite()
            || shadows.vertical_fov_radians <= 0.0
            || shadows.vertical_fov_radians >= std::f32::consts::PI
        {
            return invalid(
                "rendering.shadows.vertical_fov_radians",
                "must be finite and between zero and pi radians",
            );
        }
        ensure_finite_range(
            shadows.near_plane,
            "rendering.shadows.near_plane",
            0.0,
            MAX_VIEW_DISTANCE_METRES,
            false,
        )?;
        ensure_finite_range(
            shadows.far_plane,
            "rendering.shadows.far_plane",
            shadows.near_plane,
            MAX_VIEW_DISTANCE_METRES,
            false,
        )?;
        ensure_finite_range(
            shadows.split_lambda,
            "rendering.shadows.split_lambda",
            0.0,
            1.0,
            true,
        )?;
        ensure_integer_range(
            shadows.shadow_map_resolution,
            "rendering.shadows.shadow_map_resolution",
            1,
            MAX_SHADOW_MAP_RESOLUTION,
        )?;
        ensure_finite_range(
            shadows.direction_quantization_radians,
            "rendering.shadows.direction_quantization_radians",
            0.0,
            std::f32::consts::PI / 180.0,
            true,
        )?;
        ensure_finite_range(
            shadows.caster_depth_expansion,
            "rendering.shadows.caster_depth_expansion",
            0.0,
            shadows.far_plane,
            true,
        )?;
        let clouds = self.rendering.volumetric_clouds;
        ensure_finite_range(
            clouds.resolution_scale,
            "rendering.volumetric_clouds.resolution_scale",
            0.25,
            1.0,
            true,
        )?;
        ensure_integer_range(
            clouds.view_steps,
            "rendering.volumetric_clouds.view_steps",
            4,
            24,
        )?;
        ensure_integer_range(
            clouds.light_steps,
            "rendering.volumetric_clouds.light_steps",
            1,
            4,
        )?;
        ensure_finite_range(
            clouds.max_distance_metres,
            "rendering.volumetric_clouds.max_distance_metres",
            1_000.0,
            40_000.0,
            true,
        )?;
        ensure_finite_range(
            clouds.extinction,
            "rendering.volumetric_clouds.extinction",
            0.0001,
            0.1,
            true,
        )?;

        ensure_integer_range(
            self.diagnostics.enclosure_probe_interval_ms,
            "diagnostics.enclosure_probe_interval_ms",
            1,
            MAX_DIAGNOSTIC_INTERVAL_MS,
        )?;
        ensure_finite_range(
            self.diagnostics.enclosure_probe_distance_metres,
            "diagnostics.enclosure_probe_distance_metres",
            0.0,
            MAX_VIEW_DISTANCE_METRES,
            false,
        )?;
        ensure_finite_positive(
            self.profiling.speed_metres_per_second,
            "profiling.speed_metres_per_second",
        )?;
        ensure_finite_positive(self.profiling.warmup_seconds, "profiling.warmup_seconds")?;
        ensure_finite_non_negative(self.profiling.measure_seconds, "profiling.measure_seconds")?;
        Ok(())
    }
}

impl WorldTransportConfig {
    pub fn validate(&self) -> Result<(), ClientConfigError> {
        if !is_websocket_url(&self.endpoint) {
            return invalid(
                "world.endpoint",
                "must be an absolute ws:// or wss:// URL without whitespace",
            );
        }
        if !is_websocket_url(&self.presence_endpoint) {
            return invalid(
                "world.presence_endpoint",
                "must be an absolute ws:// or wss:// URL without whitespace",
            );
        }
        if !is_websocket_protocol_token(&self.subprotocol) {
            return invalid(
                "world.subprotocol",
                "must be a non-empty WebSocket protocol token",
            );
        }
        if !is_websocket_protocol_token(&self.auth_subprotocol_token)
            || self.auth_subprotocol_token.len() > 128
            || self.auth_subprotocol_token == self.subprotocol
        {
            return invalid(
                "world.auth_subprotocol_token",
                "must be a distinct WebSocket protocol token of at most 128 bytes",
            );
        }
        ensure_integer_range(
            self.max_in_flight_batches,
            "world.max_in_flight_batches",
            1,
            MAX_WORLD_IN_FLIGHT_BATCHES,
        )?;
        ensure_integer_range(
            self.buffered_amount_high_water_bytes,
            "world.buffered_amount_high_water_bytes",
            1,
            MAX_WORLD_BUFFERED_BYTES,
        )?;
        if self.buffered_amount_low_water_bytes >= self.buffered_amount_high_water_bytes {
            return invalid(
                "world.buffered_amount_low_water_bytes",
                "must be lower than buffered_amount_high_water_bytes",
            );
        }
        ensure_integer_range(
            self.request_timeout_ms,
            "world.request_timeout_ms",
            100,
            MAX_WORLD_REQUEST_TIMEOUT_MS,
        )?;
        ensure_integer_range(
            self.reconnect_initial_delay_ms,
            "world.reconnect_initial_delay_ms",
            10,
            MAX_WORLD_RECONNECT_DELAY_MS,
        )?;
        ensure_integer_range(
            self.reconnect_max_delay_ms,
            "world.reconnect_max_delay_ms",
            self.reconnect_initial_delay_ms,
            MAX_WORLD_RECONNECT_DELAY_MS,
        )?;
        ensure_integer_range(
            self.reconnect_attempt_limit,
            "world.reconnect_attempt_limit",
            0,
            MAX_WORLD_RECONNECT_ATTEMPTS,
        )?;
        Ok(())
    }
}

impl MultiplayerConfig {
    pub fn validate(&self) -> Result<(), ClientConfigError> {
        ensure_integer_range(
            self.pose_send_interval_ms,
            "multiplayer.pose_send_interval_ms",
            16,
            1_000,
        )?;
        ensure_integer_range(
            self.clock_sync_interval_ms,
            "multiplayer.clock_sync_interval_ms",
            250,
            60_000,
        )?;
        ensure_integer_range(
            self.buffered_amount_high_water_bytes,
            "multiplayer.buffered_amount_high_water_bytes",
            1,
            MAX_PRESENCE_BUFFERED_BYTES,
        )?;
        ensure_integer_range(
            self.min_interpolation_delay_ms,
            "multiplayer.min_interpolation_delay_ms",
            1,
            1_000,
        )?;
        ensure_integer_range(
            self.interpolation_delay_ms,
            "multiplayer.interpolation_delay_ms",
            self.min_interpolation_delay_ms,
            1_000,
        )?;
        ensure_integer_range(
            self.max_interpolation_delay_ms,
            "multiplayer.max_interpolation_delay_ms",
            self.interpolation_delay_ms,
            1_000,
        )?;
        ensure_integer_range(
            self.max_extrapolation_ms,
            "multiplayer.max_extrapolation_ms",
            0,
            1_000,
        )?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientConfigError {
    Parse(String),
    Serialize(String),
    UnsupportedSchema {
        expected: u32,
        found: u32,
    },
    InvalidValue {
        field: &'static str,
        reason: &'static str,
    },
}

impl fmt::Display for ClientConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(reason) => write!(formatter, "invalid client TOML: {reason}"),
            Self::Serialize(reason) => {
                write!(formatter, "could not serialize client TOML: {reason}")
            }
            Self::UnsupportedSchema { expected, found } => write!(
                formatter,
                "unsupported client config schema {found}; expected {expected}"
            ),
            Self::InvalidValue { field, reason } => {
                write!(formatter, "invalid client config `{field}`: {reason}")
            }
        }
    }
}

impl std::error::Error for ClientConfigError {}

fn ensure_integer_range(
    value: u32,
    field: &'static str,
    minimum: u32,
    maximum: u32,
) -> Result<(), ClientConfigError> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        invalid(field, "is outside the supported range")
    }
}

fn ensure_finite_range(
    value: f32,
    field: &'static str,
    minimum: f32,
    maximum: f32,
    include_minimum: bool,
) -> Result<(), ClientConfigError> {
    let above_minimum = if include_minimum {
        value >= minimum
    } else {
        value > minimum
    };
    if value.is_finite()
        && minimum.is_finite()
        && maximum.is_finite()
        && above_minimum
        && value <= maximum
    {
        Ok(())
    } else {
        invalid(field, "is non-finite or outside the supported range")
    }
}

fn ensure_finite_non_negative(value: f32, field: &'static str) -> Result<(), ClientConfigError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        invalid(field, "must be finite and non-negative")
    }
}

fn ensure_finite_positive(value: f32, field: &'static str) -> Result<(), ClientConfigError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        invalid(field, "must be finite and greater than zero")
    }
}

fn invalid<T>(field: &'static str, reason: &'static str) -> Result<T, ClientConfigError> {
    Err(ClientConfigError::InvalidValue { field, reason })
}

fn is_websocket_protocol_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^'..=b'`' | b'|' | b'~')
        })
}

fn is_websocket_url(value: &str) -> bool {
    (value.starts_with("ws://") || value.starts_with("wss://"))
        && !value.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMITTED_CLIENT_CONFIG: &str = include_str!("../../config/client.toml");

    fn valid_config() -> ClientConfig {
        ClientConfig {
            schema_version: CLIENT_CONFIG_SCHEMA_VERSION,
            developer: DeveloperConfig {
                controls_enabled: true,
            },
            world: WorldTransportConfig {
                endpoint: "ws://127.0.0.1:9777/v15/world".to_owned(),
                presence_endpoint: "ws://127.0.0.1:9777/v15/presence".to_owned(),
                subprotocol: "voxels.world.v15".to_owned(),
                auth_subprotocol_token: "replace-with-a-random-local-token".to_owned(),
                max_in_flight_batches: 8,
                buffered_amount_high_water_bytes: 8 * 1024 * 1024,
                buffered_amount_low_water_bytes: 2 * 1024 * 1024,
                request_timeout_ms: 10_000,
                reconnect_initial_delay_ms: 250,
                reconnect_max_delay_ms: 5_000,
                reconnect_attempt_limit: 30,
            },
            multiplayer: MultiplayerConfig {
                pose_send_interval_ms: 33,
                clock_sync_interval_ms: 1_000,
                buffered_amount_high_water_bytes: 65_536,
                interpolation_delay_ms: 100,
                min_interpolation_delay_ms: 67,
                max_interpolation_delay_ms: 200,
                max_extrapolation_ms: 750,
            },
            runtime: RuntimeConfig {
                fixed_step_seconds: 1.0 / 120.0,
                max_steps_per_frame: 6,
                max_edit_trackers: 128,
            },
            streaming: StreamingConfig {
                load_radius_chunks: 5,
                vertical_radius_chunks: 1,
                startup_ready_radius_chunks: 1,
                retention_margin_chunks: 1,
                max_tracked_chunks: 320,
                max_secondary_interest_chunks: 192,
                frame_budget: FrameBudgetConfig {
                    generation: 2,
                    meshing: 1,
                    upload: 3,
                },
                surface: SurfaceStreamingConfig {
                    load_radius_tiles: [4, 4, 4, 5, 4, 5],
                    retention_margin_tiles: 1,
                },
            },
            rendering: RenderingConfig {
                view_distance_metres: 1_000.0,
                shadows: ShadowConfig {
                    vertical_fov_radians: 68.0_f32.to_radians(),
                    near_plane: 0.05,
                    far_plane: 220.0,
                    split_lambda: 0.65,
                    shadow_map_resolution: 1_024,
                    direction_quantization_radians: std::f32::consts::PI / 5_760.0,
                    caster_depth_expansion: 64.0,
                },
                volumetric_clouds: VolumetricCloudConfig {
                    enabled: true,
                    resolution_scale: 0.5,
                    view_steps: 14,
                    light_steps: 2,
                    max_distance_metres: 14_000.0,
                    extinction: 0.006,
                },
                features: RendererFeatureConfig {
                    cascaded_sun_shadows: true,
                    voxel_ambient_occlusion: true,
                    screen_space_ambient_occlusion: false,
                    atmospheric_fog: true,
                    far_terrain: true,
                    water_surface: true,
                    target_outline: true,
                    material_surface_detail: true,
                    cave_headlamp: true,
                    voxel_emissive_lights: true,
                },
                mission_control: MissionControlConfig { open: false },
            },
            diagnostics: DiagnosticsConfig {
                enclosure_probe_interval_ms: 100,
                enclosure_probe_distance_metres: 12.0,
            },
            profiling: ProfilingConfig {
                speed_metres_per_second: 12.0,
                warmup_seconds: 30.0,
                measure_seconds: 60.0,
            },
        }
    }

    fn assert_invalid_field(config: &ClientConfig, field: &'static str) {
        assert!(matches!(
            config.validate(),
            Err(ClientConfigError::InvalidValue {
                field: actual,
                ..
            }) if actual == field
        ));
    }

    fn fixture_toml() -> String {
        valid_config().to_toml().expect("typed fixture serializes")
    }

    #[test]
    fn committed_config_is_valid_and_matches_the_explicit_runtime_fixture() {
        assert_eq!(
            ClientConfig::from_toml(COMMITTED_CLIENT_CONFIG),
            Ok(valid_config())
        );
    }

    #[test]
    fn config_round_trips_through_human_readable_toml() {
        let config = valid_config();
        let serialized = config.to_toml().expect("valid config serializes");
        assert_eq!(ClientConfig::from_toml(&serialized), Ok(config));
    }

    #[test]
    fn schema_and_unknown_fields_are_rejected() {
        let fixture = fixture_toml();
        let wrong_schema = fixture.replace("schema_version = 16", "schema_version = 15");
        assert_eq!(
            ClientConfig::from_toml(&wrong_schema),
            Err(ClientConfigError::UnsupportedSchema {
                expected: CLIENT_CONFIG_SCHEMA_VERSION,
                found: 15,
            })
        );

        let unknown_root = fixture.replace(
            "schema_version = 16",
            "schema_version = 16\nunknown_root = true",
        );
        assert!(matches!(
            ClientConfig::from_toml(&unknown_root),
            Err(ClientConfigError::Parse(_))
        ));

        let unknown_nested = fixture.replace(
            "max_edit_trackers = 128",
            "max_edit_trackers = 128\nunknown_runtime = true",
        );
        assert!(matches!(
            ClientConfig::from_toml(&unknown_nested),
            Err(ClientConfigError::Parse(_))
        ));
    }

    #[test]
    fn runtime_and_streaming_ranges_are_validated() {
        let mut config = valid_config();
        config.runtime.fixed_step_seconds = f32::NAN;
        assert_invalid_field(&config, "runtime.fixed_step_seconds");

        let mut config = valid_config();
        config.runtime.max_steps_per_frame = 0;
        assert_invalid_field(&config, "runtime.max_steps_per_frame");

        let mut config = valid_config();
        config.streaming.load_radius_chunks = MAX_SIGNED_RUNTIME_INTEGER + 1;
        assert_invalid_field(&config, "streaming.load_radius_chunks");

        let mut config = valid_config();
        config.streaming.startup_ready_radius_chunks =
            config.streaming.load_radius_chunks.saturating_add(1);
        assert_invalid_field(&config, "streaming.startup_ready_radius_chunks");

        let mut config = valid_config();
        config.streaming.max_tracked_chunks = 1;
        config.streaming.max_secondary_interest_chunks = 2;
        assert_invalid_field(&config, "streaming.max_secondary_interest_chunks");

        let mut config = valid_config();
        config.streaming.frame_budget.meshing = 0;
        assert_invalid_field(&config, "streaming.frame_budget.meshing");

        let mut config = valid_config();
        config.streaming.surface.load_radius_tiles[2] = MAX_SURFACE_RADIUS_TILES + 1;
        assert_invalid_field(&config, "streaming.surface.load_radius_tiles");
    }

    #[test]
    fn world_transport_is_provider_neutral_and_strictly_bounded() {
        let config = valid_config();
        assert_eq!(config.validate(), Ok(()));

        let serialized = config.to_toml().expect("remote transport serializes");
        assert!(!serialized.contains("procedural"));
        assert!(!serialized.contains("terrain-diffusion"));

        let mut invalid = config.clone();
        invalid.world.endpoint = "http://127.0.0.1/world".to_owned();
        assert_invalid_field(&invalid, "world.endpoint");

        let mut invalid = config.clone();
        invalid.world.subprotocol = "voxels world".to_owned();
        assert_invalid_field(&invalid, "world.subprotocol");

        let mut invalid = config.clone();
        invalid.world.auth_subprotocol_token = invalid.world.subprotocol.clone();
        assert_invalid_field(&invalid, "world.auth_subprotocol_token");

        let mut invalid = config.clone();
        invalid.world.max_in_flight_batches = 0;
        assert_invalid_field(&invalid, "world.max_in_flight_batches");

        let mut invalid = config.clone();
        invalid.world.buffered_amount_low_water_bytes =
            invalid.world.buffered_amount_high_water_bytes;
        assert_invalid_field(&invalid, "world.buffered_amount_low_water_bytes");

        let mut invalid = config.clone();
        invalid.world.request_timeout_ms = 99;
        assert_invalid_field(&invalid, "world.request_timeout_ms");

        let mut invalid = config;
        invalid.world.reconnect_max_delay_ms =
            invalid.world.reconnect_initial_delay_ms.saturating_sub(1);
        assert_invalid_field(&invalid, "world.reconnect_max_delay_ms");
    }

    #[test]
    fn rendering_ranges_are_validated() {
        let mut config = valid_config();
        config.rendering.view_distance_metres = f32::INFINITY;
        assert_invalid_field(&config, "rendering.view_distance_metres");

        let mut config = valid_config();
        config.rendering.shadows.vertical_fov_radians = std::f32::consts::PI;
        assert_invalid_field(&config, "rendering.shadows.vertical_fov_radians");

        let mut config = valid_config();
        config.rendering.shadows.far_plane = config.rendering.shadows.near_plane;
        assert_invalid_field(&config, "rendering.shadows.far_plane");

        let mut config = valid_config();
        config.rendering.shadows.split_lambda = 1.01;
        assert_invalid_field(&config, "rendering.shadows.split_lambda");

        let mut config = valid_config();
        config.rendering.shadows.shadow_map_resolution = MAX_SHADOW_MAP_RESOLUTION + 1;
        assert_invalid_field(&config, "rendering.shadows.shadow_map_resolution");

        let mut config = valid_config();
        config.rendering.volumetric_clouds.resolution_scale = 0.1;
        assert_invalid_field(&config, "rendering.volumetric_clouds.resolution_scale");

        let mut config = valid_config();
        config.rendering.volumetric_clouds.view_steps = 25;
        assert_invalid_field(&config, "rendering.volumetric_clouds.view_steps");
    }

    #[test]
    fn diagnostics_and_profiling_ranges_are_validated() {
        let mut config = valid_config();
        config.profiling.warmup_seconds = -1.0;
        assert_invalid_field(&config, "profiling.warmup_seconds");

        let mut config = valid_config();
        config.profiling.speed_metres_per_second = 0.0;
        assert_invalid_field(&config, "profiling.speed_metres_per_second");

        let mut config = valid_config();
        config.profiling.warmup_seconds = 0.0;
        assert_invalid_field(&config, "profiling.warmup_seconds");

        let mut config = valid_config();
        config.profiling.measure_seconds = f32::NAN;
        assert_invalid_field(&config, "profiling.measure_seconds");

        let mut config = valid_config();
        config.profiling.measure_seconds = 0.0;
        assert_eq!(config.validate(), Ok(()));
    }
}
