//! Portable state, layout, hit-testing, and draw-list generation for the in-canvas mission control.
//!
//! Coordinates in this module are CSS/device-independent pixels. The browser shell may forward raw
//! device pixels through [`Viewport::device_to_css`], but the state and resulting draw list contain
//! no browser, WASM, WGPU, or resource-lifetime concerns.

use std::{collections::BTreeMap, fmt::Write};

const PANEL_INSET: f32 = 18.0;
const PANEL_WIDTH: f32 = 520.0;
const COMPACT_WIDTH: f32 = 320.0;
const HEADER_HEIGHT: f32 = 82.0;
const NAVIGATION_HEIGHT: f32 = 92.0;
const COMPACT_NAVIGATION_HEIGHT: f32 = 108.0;
const PANEL_RADIUS: f32 = 18.0;
const CONTENT_PAD: f32 = 14.0;
const BUTTON_SIZE: f32 = 30.0;
const BUTTON_GAP: f32 = 6.0;
const ACTION_BUTTON_WIDTH: f32 = 40.0;
const CHROME_HEIGHT: f32 = 36.0;
const LAUNCHER_WIDTH: f32 = 222.0;
const INVENTORY_WIDTH: f32 = 390.0;
const INVENTORY_HEIGHT: f32 = 108.0;
const CHROME_STACK_GAP: f32 = 8.0;
const PANEL_TOP: f32 = PANEL_INSET + CHROME_HEIGHT + 12.0;
const AUTO_COMPACT_WIDTH: f32 = 720.0;
const MOTION_RATE: f32 = 18.0;
const TOAST_HOLD_SECONDS: f32 = 5.0;
const TOAST_FADE_SECONDS: f32 = 1.5;

const PANEL_COLOR: Color = Color::new(0.055, 0.070, 0.105, 0.88);
const PANEL_BORDER: Color = Color::new(0.52, 0.66, 0.88, 0.28);
const CARD_COLOR: Color = Color::new(0.080, 0.100, 0.145, 0.72);
const HOVER_COLOR: Color = Color::new(0.34, 0.62, 0.94, 0.20);
const TEXT_PRIMARY: Color = Color::new(0.94, 0.96, 1.0, 1.0);
const TEXT_MUTED: Color = Color::new(0.62, 0.69, 0.80, 1.0);
const ACCENT: Color = Color::new(0.28, 0.76, 0.96, 1.0);
const TOGGLE_OFF: Color = Color::new(0.16, 0.19, 0.25, 0.96);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self([red, green, blue, alpha])
    }

    fn with_alpha(self, multiplier: f32) -> Self {
        Self([self.0[0], self.0[1], self.0[2], self.0[3] * multiplier])
    }

    fn mix(self, other: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        Self([
            self.0[0] + (other.0[0] - self.0[0]) * amount,
            self.0[1] + (other.0[1] - self.0[1]) * amount,
            self.0[2] + (other.0[2] - self.0[2]) * amount,
            self.0[3] + (other.0[3] - self.0[3]) * amount,
        ])
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.x
            && point[0] <= self.x + self.width
            && point[1] >= self.y
            && point[1] <= self.y + self.height
    }

    pub const fn center(self) -> [f32; 2] {
        [self.x + self.width * 0.5, self.y + self.height * 0.5]
    }
}

/// Device-pixel viewport plus its density conversion. Invalid density values gracefully become 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub device_width: f32,
    pub device_height: f32,
    pub scale_factor: f32,
}

impl Viewport {
    pub fn new(device_width: f32, device_height: f32, scale_factor: f32) -> Self {
        Self {
            device_width: device_width.max(0.0),
            device_height: device_height.max(0.0),
            scale_factor: if scale_factor.is_finite() && scale_factor > 0.0 {
                scale_factor
            } else {
                1.0
            },
        }
    }

    pub fn css_size(self) -> [f32; 2] {
        [
            self.device_width / self.scale_factor,
            self.device_height / self.scale_factor,
        ]
    }

    pub fn device_to_css(self, point: [f32; 2]) -> [f32; 2] {
        [point[0] / self.scale_factor, point[1] / self.scale_factor]
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RendererFeature {
    CascadedSunShadows,
    VoxelAmbientOcclusion,
    ScreenSpaceAmbientOcclusion,
    AtmosphericFog,
    FarTerrain,
    WaterSurface,
    TargetOutline,
    MaterialSurfaceDetail,
    CaveHeadlamp,
    VoxelEmissiveLights,
}

impl RendererFeature {
    pub const ALL: [Self; 10] = [
        Self::CascadedSunShadows,
        Self::VoxelAmbientOcclusion,
        Self::ScreenSpaceAmbientOcclusion,
        Self::AtmosphericFog,
        Self::FarTerrain,
        Self::WaterSurface,
        Self::TargetOutline,
        Self::MaterialSurfaceDetail,
        Self::CaveHeadlamp,
        Self::VoxelEmissiveLights,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::CascadedSunShadows => "Cascaded sun shadows",
            Self::VoxelAmbientOcclusion => "Voxel ambient occlusion",
            Self::ScreenSpaceAmbientOcclusion => "Screen-space contact AO",
            Self::AtmosphericFog => "Atmospheric fog",
            Self::FarTerrain => "Far terrain",
            Self::WaterSurface => "Animated water surface",
            Self::TargetOutline => "Target outline",
            Self::MaterialSurfaceDetail => "Material surface detail",
            Self::CaveHeadlamp => "Automatic cave headlamp",
            Self::VoxelEmissiveLights => "Voxel emissive lights",
        }
    }

    pub const fn compact_label(self) -> &'static str {
        match self {
            Self::CascadedSunShadows => "Sun shadow",
            Self::VoxelAmbientOcclusion => "Voxel AO",
            Self::ScreenSpaceAmbientOcclusion => "Contact AO",
            Self::AtmosphericFog => "Fog",
            Self::FarTerrain => "Far terrain",
            Self::WaterSurface => "Water",
            Self::TargetOutline => "Target",
            Self::MaterialSurfaceDetail => "Surface detail",
            Self::CaveHeadlamp => "Headlamp",
            Self::VoxelEmissiveLights => "Voxel lights",
        }
    }
}

/// Configured baseline for every renderer feature exposed by Mission Control.
///
/// The renderer and UI consume the same value so a toggle always describes actual renderer state,
/// and "reset" means restore the host-provided baseline rather than enable every feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

impl RendererFeatureConfig {
    pub const fn enabled(self, feature: RendererFeature) -> bool {
        match feature {
            RendererFeature::CascadedSunShadows => self.cascaded_sun_shadows,
            RendererFeature::VoxelAmbientOcclusion => self.voxel_ambient_occlusion,
            RendererFeature::ScreenSpaceAmbientOcclusion => self.screen_space_ambient_occlusion,
            RendererFeature::AtmosphericFog => self.atmospheric_fog,
            RendererFeature::FarTerrain => self.far_terrain,
            RendererFeature::WaterSurface => self.water_surface,
            RendererFeature::TargetOutline => self.target_outline,
            RendererFeature::MaterialSurfaceDetail => self.material_surface_detail,
            RendererFeature::CaveHeadlamp => self.cave_headlamp,
            RendererFeature::VoxelEmissiveLights => self.voxel_emissive_lights,
        }
    }
}

impl Default for RendererFeatureConfig {
    fn default() -> Self {
        Self {
            cascaded_sun_shadows: true,
            voxel_ambient_occlusion: true,
            screen_space_ambient_occlusion: true,
            atmospheric_fog: true,
            far_terrain: true,
            water_surface: true,
            target_outline: true,
            material_surface_detail: true,
            cave_headlamp: true,
            voxel_emissive_lights: true,
        }
    }
}

/// Initial presentation state for the in-canvas Mission Control panel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MissionControlConfig {
    pub open: bool,
    pub compact: bool,
}

const FEATURE_COUNT: usize = RendererFeature::ALL.len();
const PANEL_HEIGHT: f32 = 680.0;
const COMPACT_HEIGHT: f32 = 750.0;

const fn feature_index(feature: RendererFeature) -> usize {
    match feature {
        RendererFeature::CascadedSunShadows => 0,
        RendererFeature::VoxelAmbientOcclusion => 1,
        RendererFeature::ScreenSpaceAmbientOcclusion => 2,
        RendererFeature::AtmosphericFog => 3,
        RendererFeature::FarTerrain => 4,
        RendererFeature::WaterSurface => 5,
        RendererFeature::TargetOutline => 6,
        RendererFeature::MaterialSurfaceDetail => 7,
        RendererFeature::CaveHeadlamp => 8,
        RendererFeature::VoxelEmissiveLights => 9,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum UiTarget {
    Launcher,
    Header,
    CopyDiagnostics,
    ResetRendererFeatures,
    Close,
    Compact,
    Feature(RendererFeature),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiKey {
    F3,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    None,
    CopyDiagnostics,
    ResetRendererFeatures,
    PanelOpenChanged(bool),
    CompactChanged(bool),
    FeatureChanged(RendererFeature, bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavigationTelemetry {
    /// Player eye position in metres.
    pub eye_position_metres: [f32; 3],
    /// Canonical 10 cm voxel containing the player eye position.
    pub eye_voxel: [i32; 3],
    /// Canonical chunk containing `eye_voxel`.
    pub eye_chunk: [i32; 3],
    /// Compass heading where 0 is north (-Z) and 90 is east (+X).
    pub heading_degrees: f32,
    /// Positive looks up and negative looks down.
    pub pitch_degrees: f32,
    pub horizontal_speed_metres_per_second: f32,
    pub grounded: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LiveStats {
    pub navigation: NavigationTelemetry,
    pub frames_per_second: f32,
    pub frame_ms: f32,
    pub cpu_ms: f32,
    pub gpu_ms: Option<f32>,
    pub gpu_ambient_occlusion_ms: Option<f32>,
    pub resident_chunks: u32,
    pub visible_chunks: u32,
    pub quads: u32,
    pub water_quads: u32,
    pub draw_calls: u32,
    pub water_draw_calls: u32,
    pub shadow_draw_calls: u32,
    pub shadow_cascades: u32,
    pub load_p95_frames: u64,
    pub load_max_frames: u64,
    pub remesh_p95_frames: u64,
    pub remesh_max_frames: u64,
    pub edit_last_ms: f32,
    pub edit_in_flight: u32,
    pub lod_tiles: [u32; voxels_world::SURFACE_LOD_LEVEL_COUNT],
    pub pending_jobs: u32,
    pub core_gpu_bytes: u64,
    pub water_immersion: f32,
    pub eye_depth_metres: f32,
    pub eyes_submerged: bool,
    pub swimming: bool,
    pub local_light_candidates: u32,
    pub active_local_lights: u32,
    pub occluded_local_lights: u32,
    pub portal_rejected_local_lights: u32,
    pub open_cinder_portals: u32,
    pub cinder_portal_revision: u32,
    pub stream_interest_requested: u32,
    pub stream_interest_desired: u32,
    pub stream_interest_truncated: u32,
    pub portal_active_chunks: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceRole {
    Launcher,
    Inventory,
    InventoryOrb,
    Toast,
    Crosshair,
    Panel,
    Header,
    NavigationCard,
    Button,
    StatCard,
    FeatureRow,
    ToggleTrack,
    ToggleThumb,
}

/// Renderer-agnostic description of a translucent rounded surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassSurface {
    pub rect: Rect,
    pub radius: f32,
    pub fill: Color,
    pub border: Color,
    pub role: SurfaceRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Renderer-agnostic text command. Glyph caching and shaping remain renderer responsibilities.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub position: [f32; 2],
    pub size: f32,
    pub color: Color,
    pub align: TextAlign,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiDrawList {
    pub glass: Vec<GlassSurface>,
    pub text: Vec<TextRun>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractiveRegion {
    pub target: UiTarget,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiLayout {
    pub launcher: Rect,
    pub inventory: Rect,
    pub toast: Rect,
    pub crosshair: Rect,
    pub panel: Rect,
    pub header: Rect,
    pub navigation: Rect,
    pub compact: bool,
    pub regions: Vec<InteractiveRegion>,
    pub stat_cards: Vec<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InventoryItem {
    pub label: &'static str,
    pub count: u64,
    pub color: Color,
}

impl UiLayout {
    pub fn region(&self, target: UiTarget) -> Option<Rect> {
        self.regions
            .iter()
            .find(|region| region.target == target)
            .map(|region| region.rect)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EasedValue {
    value: f32,
    target: f32,
}

impl EasedValue {
    const fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
        }
    }

    fn set(&mut self, target: bool, reduced_motion: bool) {
        self.target = f32::from(target);
        if reduced_motion {
            self.value = self.target;
        }
    }

    fn advance(&mut self, dt: f32, reduced_motion: bool) {
        if reduced_motion {
            self.value = self.target;
            return;
        }
        let amount = 1.0 - (-MOTION_RATE * dt.clamp(0.0, 0.1)).exp();
        self.value += (self.target - self.value) * amount;
        if (self.target - self.value).abs() < 0.000_5 {
            self.value = self.target;
        }
    }
}

/// Owns developer-surface state while leaving renderer features and actions under host control.
pub struct MissionControlUi {
    open: bool,
    compact: bool,
    reduced_motion: bool,
    hovered: Option<UiTarget>,
    stats: LiveStats,
    feature_enabled: [bool; FEATURE_COUNT],
    feature_motion: [EasedValue; FEATURE_COUNT],
    open_motion: EasedValue,
    hover_motion: BTreeMap<UiTarget, EasedValue>,
    toast_age: f32,
    gameplay_toast: Option<String>,
    daylight_label: &'static str,
    world_time_label: String,
    weather_label: String,
    region_label: &'static str,
    route_chapter_label: &'static str,
    route_progress_percent: u8,
    placement_material_available: bool,
    placement_material_label: &'static str,
    placement_material_count: u64,
    inventory_summary: [String; 2],
    inventory_items: Vec<InventoryItem>,
    selected_inventory_index: Option<usize>,
}

impl Default for MissionControlUi {
    fn default() -> Self {
        Self::new(
            MissionControlConfig::default(),
            RendererFeatureConfig::default(),
        )
    }
}

impl MissionControlUi {
    pub fn new(config: MissionControlConfig, features: RendererFeatureConfig) -> Self {
        let feature_enabled =
            std::array::from_fn(|index| features.enabled(RendererFeature::ALL[index]));
        let feature_motion =
            std::array::from_fn(|index| EasedValue::new(f32::from(feature_enabled[index])));
        Self {
            open: config.open,
            compact: config.compact,
            reduced_motion: false,
            hovered: None,
            stats: LiveStats::default(),
            feature_enabled,
            feature_motion,
            open_motion: EasedValue::new(f32::from(config.open)),
            hover_motion: BTreeMap::new(),
            toast_age: 0.0,
            gameplay_toast: None,
            daylight_label: "GOLDEN HOUR",
            world_time_label: "17:17".to_owned(),
            weather_label: "CLEAR · CLOUDS 24% · WIND 0.0, 0.0 M/S · R1".to_owned(),
            region_label: "VERDANT FOREST",
            route_chapter_label: "OFF PILGRIM ROAD",
            route_progress_percent: 0,
            placement_material_available: false,
            placement_material_label: "",
            placement_material_count: 0,
            inventory_summary: [String::new(), String::new()],
            inventory_items: Vec::new(),
            selected_inventory_index: None,
        }
    }

    pub const fn open(&self) -> bool {
        self.open
    }

    pub const fn compact(&self) -> bool {
        self.compact
    }

    pub const fn stats(&self) -> LiveStats {
        self.stats
    }

    pub fn set_stats(&mut self, stats: LiveStats) {
        if stats.swimming && !self.stats.swimming {
            self.gameplay_toast = None;
            self.toast_age = 0.0;
        }
        self.stats = stats;
    }

    pub fn set_environment_status(
        &mut self,
        daylight_label: &'static str,
        region_label: &'static str,
    ) {
        self.daylight_label = daylight_label;
        self.region_label = region_label;
    }

    pub fn set_world_clock(
        &mut self,
        day_fraction: f32,
        weather_label: &'static str,
        precipitation: f32,
        cloud_coverage: f32,
        cloud_velocity_metres_per_second: [f32; 2],
        weather_revision: u64,
    ) {
        let total_minutes = (day_fraction.rem_euclid(1.0) * 1_440.0).round() as u32 % 1_440;
        self.world_time_label = format!("{:02}:{:02}", total_minutes / 60, total_minutes % 60);
        self.weather_label = format!(
            "{} {:.0}% · CLOUDS {:.0}% · WIND {:.1}, {:.1} M/S · R{}",
            weather_label,
            precipitation.clamp(0.0, 1.0) * 100.0,
            cloud_coverage.clamp(0.0, 1.0) * 100.0,
            cloud_velocity_metres_per_second[0],
            cloud_velocity_metres_per_second[1],
            weather_revision,
        );
    }

    pub fn set_route_status(&mut self, chapter_label: &'static str, progress_percent: u8) {
        self.route_chapter_label = chapter_label;
        self.route_progress_percent = progress_percent.min(100);
    }

    pub fn set_inventory(
        &mut self,
        selected_label: Option<&'static str>,
        selected_count: u64,
        summary: [String; 2],
        items: Vec<InventoryItem>,
        selected_index: Option<usize>,
    ) {
        self.placement_material_available = selected_label.is_some();
        self.placement_material_label = selected_label.unwrap_or("");
        self.placement_material_count = selected_count;
        self.inventory_summary = summary;
        self.inventory_items = items;
        self.selected_inventory_index =
            selected_index.filter(|index| *index < self.inventory_items.len());
    }

    pub fn show_gameplay_toast(&mut self, message: impl Into<String>) {
        self.gameplay_toast = Some(message.into());
        self.toast_age = 0.0;
    }

    pub fn set_reduced_motion(&mut self, reduced_motion: bool) {
        self.reduced_motion = reduced_motion;
        if reduced_motion {
            self.snap_motion();
        }
    }

    pub fn set_open(&mut self, open: bool) -> UiAction {
        if self.open == open {
            return UiAction::None;
        }
        self.open = open;
        self.open_motion.set(open, self.reduced_motion);
        if !open {
            self.set_hover(None);
        }
        UiAction::PanelOpenChanged(open)
    }

    pub fn toggle_open(&mut self) -> UiAction {
        self.set_open(!self.open)
    }

    /// F3 toggles only on the initial key-down, avoiding repeat flicker while the key is held.
    pub fn handle_key(&mut self, key: UiKey, pressed: bool, repeat: bool) -> UiAction {
        if key == UiKey::F3 && pressed && !repeat {
            self.toggle_open()
        } else {
            UiAction::None
        }
    }

    pub fn set_compact(&mut self, compact: bool) -> UiAction {
        if self.compact == compact {
            return UiAction::None;
        }
        self.compact = compact;
        UiAction::CompactChanged(compact)
    }

    pub const fn feature_enabled(&self, feature: RendererFeature) -> bool {
        self.feature_enabled[feature_index(feature)]
    }

    pub fn set_feature(&mut self, feature: RendererFeature, enabled: bool) -> UiAction {
        let index = feature_index(feature);
        if self.feature_enabled[index] == enabled {
            return UiAction::None;
        }
        self.feature_enabled[index] = enabled;
        self.feature_motion[index].set(enabled, self.reduced_motion);
        UiAction::FeatureChanged(feature, enabled)
    }

    pub fn toggle_feature(&mut self, feature: RendererFeature) -> UiAction {
        self.set_feature(feature, !self.feature_enabled(feature))
    }

    pub fn feature_eased_value(&self, feature: RendererFeature) -> f32 {
        self.feature_motion[feature_index(feature)].value
    }

    pub fn hover_eased_value(&self, target: UiTarget) -> f32 {
        self.hover_motion
            .get(&target)
            .map_or(0.0, |motion| motion.value)
    }

    pub fn advance(&mut self, dt: f32) {
        self.toast_age += dt.clamp(0.0, 0.1);
        self.open_motion.advance(dt, self.reduced_motion);
        for motion in &mut self.feature_motion {
            motion.advance(dt, self.reduced_motion);
        }
        for motion in self.hover_motion.values_mut() {
            motion.advance(dt, self.reduced_motion);
        }
    }

    pub fn effective_compact(&self, viewport: Viewport) -> bool {
        self.compact || viewport.css_size()[0] < AUTO_COMPACT_WIDTH
    }

    pub fn layout(&self, viewport: Viewport) -> UiLayout {
        let [viewport_width, viewport_height] = viewport.css_size();
        let automatically_compact = viewport_width < AUTO_COMPACT_WIDTH;
        let compact = self.effective_compact(viewport);
        let requested_width = if compact { COMPACT_WIDTH } else { PANEL_WIDTH };
        let panel_width = requested_width.min((viewport_width - PANEL_INSET * 2.0).max(180.0));
        let launcher_y = PANEL_INSET;
        let panel_top = PANEL_TOP;
        let panel_height = if compact {
            COMPACT_HEIGHT
        } else {
            PANEL_HEIGHT
        }
        .min((viewport_height - panel_top - PANEL_INSET).max(HEADER_HEIGHT));
        let launcher = Rect::new(
            (viewport_width - PANEL_INSET - LAUNCHER_WIDTH).max(0.0),
            launcher_y,
            LAUNCHER_WIDTH.min(viewport_width.max(0.0)),
            CHROME_HEIGHT,
        );
        let crosshair_center = [viewport_width * 0.5, viewport_height * 0.5];
        let toast_width = 590.0f32.min((viewport_width - PANEL_INSET * 2.0).max(180.0));
        let toast = Rect::new(
            (viewport_width - toast_width) * 0.5,
            (viewport_height - CHROME_HEIGHT - PANEL_INSET).max(0.0),
            toast_width,
            CHROME_HEIGHT,
        );
        let inventory_width = INVENTORY_WIDTH.min((viewport_width - PANEL_INSET * 2.0).max(180.0));
        let inventory = Rect::new(
            (viewport_width - inventory_width) * 0.5,
            (toast.y - CHROME_STACK_GAP - INVENTORY_HEIGHT).max(0.0),
            inventory_width,
            INVENTORY_HEIGHT,
        );
        let crosshair = Rect::new(
            crosshair_center[0] - 6.0,
            crosshair_center[1] - 6.0,
            12.0,
            12.0,
        );
        let panel = Rect::new(
            (viewport_width - panel_width - PANEL_INSET).max(0.0),
            panel_top.min((viewport_height - HEADER_HEIGHT).max(0.0)),
            panel_width,
            panel_height,
        );
        let header = Rect::new(
            panel.x,
            panel.y,
            panel.width,
            HEADER_HEIGHT.min(panel.height),
        );
        let mut regions = vec![
            InteractiveRegion {
                target: UiTarget::Launcher,
                rect: launcher,
            },
            InteractiveRegion {
                target: UiTarget::Header,
                rect: header,
            },
        ];

        // Keep actions on the title row so environment and inventory status always have the full
        // panel width below them. The previous vertically centred row overlapped both status lines.
        let button_y = header.y + 7.0;
        let close = Rect::new(
            panel.x + panel.width - CONTENT_PAD - BUTTON_SIZE,
            button_y,
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        let reset = Rect::new(
            close.x - BUTTON_GAP - ACTION_BUTTON_WIDTH,
            button_y,
            ACTION_BUTTON_WIDTH,
            BUTTON_SIZE,
        );
        let compact_button = Rect::new(
            reset.x - BUTTON_GAP - BUTTON_SIZE,
            button_y,
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        let copy_right = if automatically_compact {
            reset.x
        } else {
            compact_button.x
        };
        let copy = Rect::new(
            copy_right - BUTTON_GAP - ACTION_BUTTON_WIDTH,
            button_y,
            ACTION_BUTTON_WIDTH,
            BUTTON_SIZE,
        );
        regions.extend([
            InteractiveRegion {
                target: UiTarget::CopyDiagnostics,
                rect: copy,
            },
            InteractiveRegion {
                target: UiTarget::ResetRendererFeatures,
                rect: reset,
            },
            InteractiveRegion {
                target: UiTarget::Close,
                rect: close,
            },
        ]);
        if !automatically_compact {
            regions.push(InteractiveRegion {
                target: UiTarget::Compact,
                rect: compact_button,
            });
        }

        let navigation = Rect::new(
            panel.x + CONTENT_PAD,
            panel.y + HEADER_HEIGHT + 12.0,
            panel.width - CONTENT_PAD * 2.0,
            if compact {
                COMPACT_NAVIGATION_HEIGHT
            } else {
                NAVIGATION_HEIGHT
            },
        );
        let stats_top = navigation.y + navigation.height + 24.0;
        let stat_columns: usize = 2;
        let stat_gap = 7.0;
        let stat_width = (panel.width - CONTENT_PAD * 2.0 - stat_gap * (stat_columns - 1) as f32)
            / stat_columns as f32;
        let stat_height = if compact { 44.0 } else { 50.0 };
        let dense = compact || panel.height < 620.0;
        let stat_count: usize = if dense { 6 } else { 8 };
        let stat_cards = (0..stat_count)
            .map(|index| {
                let column = index % stat_columns;
                let row = index / stat_columns;
                Rect::new(
                    panel.x + CONTENT_PAD + column as f32 * (stat_width + stat_gap),
                    stats_top + row as f32 * (stat_height + stat_gap),
                    stat_width,
                    stat_height,
                )
            })
            .collect::<Vec<_>>();

        let stats_rows = stat_count.div_ceil(stat_columns);
        let feature_top = stats_top + stats_rows as f32 * (stat_height + stat_gap) + 23.0;
        let feature_columns: usize = 2;
        let feature_rows = FEATURE_COUNT.div_ceil(feature_columns);
        let feature_gap = 7.0;
        let feature_width =
            (panel.width - CONTENT_PAD * 2.0 - feature_gap * (feature_columns - 1) as f32)
                / feature_columns as f32;
        let nominal_feature_row_height: f32 = 38.0;
        let available_feature_height =
            (panel.y + panel.height - CONTENT_PAD - feature_top).max(0.0);
        let feature_row_height = nominal_feature_row_height
            .min(available_feature_height / feature_rows.max(1) as f32)
            .max(1.0);
        for (index, feature) in RendererFeature::ALL.into_iter().enumerate() {
            let column = index % feature_columns;
            let row = index / feature_columns;
            regions.push(InteractiveRegion {
                target: UiTarget::Feature(feature),
                rect: Rect::new(
                    panel.x + CONTENT_PAD + column as f32 * (feature_width + feature_gap),
                    feature_top + row as f32 * feature_row_height,
                    feature_width,
                    feature_row_height,
                ),
            });
        }

        UiLayout {
            launcher,
            inventory,
            toast,
            crosshair,
            panel,
            header,
            navigation,
            compact,
            regions,
            stat_cards,
        }
    }

    pub fn hit_test_css(&self, point: [f32; 2], viewport: Viewport) -> Option<UiTarget> {
        let layout = self.layout(viewport);
        if !self.open {
            return layout
                .regions
                .iter()
                .find(|region| region.target == UiTarget::Launcher && region.rect.contains(point))
                .map(|region| region.target);
        }
        layout
            .regions
            .iter()
            .rev()
            .find(|region| region.rect.contains(point))
            .map(|region| region.target)
    }

    pub fn inventory_contains_css(&self, point: [f32; 2], viewport: Viewport) -> bool {
        self.layout(viewport).inventory.contains(point)
    }

    pub fn hit_test_device(&self, point: [f32; 2], viewport: Viewport) -> Option<UiTarget> {
        self.hit_test_css(viewport.device_to_css(point), viewport)
    }

    pub fn pointer_move_device(&mut self, point: [f32; 2], viewport: Viewport) -> bool {
        let target = self.hit_test_device(point, viewport);
        if target == self.hovered {
            return false;
        }
        self.set_hover(target);
        true
    }

    pub fn activate_device(&mut self, point: [f32; 2], viewport: Viewport) -> UiAction {
        self.activate_css(viewport.device_to_css(point), viewport)
    }

    pub fn activate_css(&mut self, point: [f32; 2], viewport: Viewport) -> UiAction {
        if self.hit_test_css(point, viewport) == Some(UiTarget::Launcher) {
            return self.toggle_open();
        }
        if !self.open {
            return UiAction::None;
        }
        match self.hit_test_css(point, viewport) {
            Some(UiTarget::Launcher) => self.toggle_open(),
            Some(UiTarget::CopyDiagnostics) => UiAction::CopyDiagnostics,
            Some(UiTarget::ResetRendererFeatures) => UiAction::ResetRendererFeatures,
            Some(UiTarget::Close) => self.set_open(false),
            Some(UiTarget::Compact) => self.set_compact(!self.compact),
            Some(UiTarget::Feature(feature)) => self.toggle_feature(feature),
            Some(UiTarget::Header) | None => UiAction::None,
        }
    }

    pub fn build_draw_list(&self, viewport: Viewport) -> UiDrawList {
        let layout = self.layout(viewport);
        let mut draw = UiDrawList::default();
        self.push_chrome(&mut draw, &layout);
        if !self.open && self.open_motion.value <= 0.000_5 {
            return draw;
        }
        let opacity = self.open_motion.value.clamp(0.0, 1.0);
        push_surface(
            &mut draw,
            layout.panel,
            PANEL_RADIUS,
            PANEL_COLOR.with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity),
            SurfaceRole::Panel,
        );
        push_surface(
            &mut draw,
            layout.header,
            PANEL_RADIUS,
            CARD_COLOR.with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.65),
            SurfaceRole::Header,
        );
        push_text(
            &mut draw,
            if layout.compact {
                "MISSION CONTROL"
            } else {
                "VOXELS / MISSION CONTROL"
            },
            [layout.panel.x + CONTENT_PAD, layout.header.y + 21.0],
            if layout.compact { 10.0 } else { 11.5 },
            TEXT_PRIMARY.with_alpha(opacity),
            TextAlign::Left,
        );
        push_text(
            &mut draw,
            format!(
                "{} · {} / {}",
                self.world_time_label, self.daylight_label, self.region_label
            ),
            [layout.panel.x + CONTENT_PAD, layout.header.y + 48.0],
            if layout.compact { 8.0 } else { 9.0 },
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );
        let placement_status = if self.placement_material_available {
            format!(
                "PLACE {} ×{}",
                self.placement_material_label,
                compact_count(self.placement_material_count),
            )
        } else {
            "INVENTORY EMPTY · DIG TO COLLECT".to_owned()
        };
        push_text(
            &mut draw,
            placement_status,
            [layout.panel.x + CONTENT_PAD, layout.header.y + 66.0],
            if layout.compact { 9.0 } else { 10.0 },
            ACCENT.with_alpha(opacity),
            TextAlign::Left,
        );

        for (target, label) in [
            (UiTarget::CopyDiagnostics, "COPY"),
            (UiTarget::ResetRendererFeatures, "RESET"),
            (UiTarget::Compact, if layout.compact { ">" } else { "<" }),
            (UiTarget::Close, "x"),
        ] {
            if let Some(rect) = layout.region(target) {
                let hover = self.hover_eased_value(target);
                push_surface(
                    &mut draw,
                    rect,
                    8.0,
                    CARD_COLOR.mix(HOVER_COLOR, hover).with_alpha(opacity),
                    PANEL_BORDER.with_alpha(opacity * (0.45 + hover * 0.55)),
                    SurfaceRole::Button,
                );
                push_text(
                    &mut draw,
                    label,
                    rect.center(),
                    if matches!(
                        target,
                        UiTarget::CopyDiagnostics | UiTarget::ResetRendererFeatures
                    ) {
                        8.0
                    } else {
                        12.0
                    },
                    TEXT_PRIMARY.with_alpha(opacity),
                    TextAlign::Center,
                );
            }
        }

        self.push_navigation(&mut draw, &layout, opacity);

        let card_data = self.card_data(layout.compact, layout.stat_cards.len());
        for (rect, (label, value)) in layout.stat_cards.iter().copied().zip(card_data) {
            push_surface(
                &mut draw,
                rect,
                10.0,
                CARD_COLOR.with_alpha(opacity),
                PANEL_BORDER.with_alpha(opacity * 0.35),
                SurfaceRole::StatCard,
            );
            push_text(
                &mut draw,
                label,
                [
                    rect.x + 9.0,
                    rect.y + if layout.compact { 13.0 } else { 15.0 },
                ],
                if layout.compact { 8.0 } else { 9.0 },
                TEXT_MUTED.with_alpha(opacity),
                TextAlign::Left,
            );
            push_text(
                &mut draw,
                value,
                [
                    rect.x + 9.0,
                    rect.y + if layout.compact { 31.0 } else { 36.0 },
                ],
                if layout.compact { 11.5 } else { 13.0 },
                TEXT_PRIMARY.with_alpha(opacity),
                TextAlign::Left,
            );
        }

        let first_feature_y = layout
            .region(UiTarget::Feature(RendererFeature::CascadedSunShadows))
            .map_or(layout.header.y + HEADER_HEIGHT, |rect| rect.y);
        push_text(
            &mut draw,
            "RENDER FEATURES",
            [layout.panel.x + CONTENT_PAD, first_feature_y - 11.0],
            9.0,
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );
        for feature in RendererFeature::ALL {
            let target = UiTarget::Feature(feature);
            let Some(rect) = layout.region(target) else {
                continue;
            };
            let hover = self.hover_eased_value(target);
            push_surface(
                &mut draw,
                rect,
                8.0,
                CARD_COLOR
                    .mix(HOVER_COLOR, hover)
                    .with_alpha(opacity * (0.56 + hover * 0.30)),
                PANEL_BORDER.with_alpha(opacity * hover * 0.8),
                SurfaceRole::FeatureRow,
            );
            push_text(
                &mut draw,
                if layout.compact {
                    feature.compact_label()
                } else {
                    feature.label()
                },
                [rect.x + 10.0, rect.center()[1]],
                if layout.compact { 9.5 } else { 11.5 },
                TEXT_PRIMARY.with_alpha(opacity),
                TextAlign::Left,
            );
            self.push_toggle(&mut draw, rect, feature, opacity);
        }
        draw
    }

    fn push_navigation(&self, draw: &mut UiDrawList, layout: &UiLayout, opacity: f32) {
        let navigation = self.stats.navigation;
        push_surface(
            draw,
            layout.navigation,
            12.0,
            CARD_COLOR.with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.55),
            SurfaceRole::NavigationCard,
        );
        push_text(
            draw,
            "PLAYER NAVIGATION",
            [layout.navigation.x + 11.0, layout.navigation.y + 16.0],
            9.0,
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );
        let [x, y, z] = navigation.eye_position_metres;
        push_text(
            draw,
            format!("X {x:.2}   Y {y:.2}   Z {z:.2} m"),
            [layout.navigation.x + 11.0, layout.navigation.y + 38.0],
            if layout.compact { 13.0 } else { 16.0 },
            TEXT_PRIMARY.with_alpha(opacity),
            TextAlign::Left,
        );
        let [voxel_x, voxel_y, voxel_z] = navigation.eye_voxel;
        let [chunk_x, chunk_y, chunk_z] = navigation.eye_chunk;
        let movement = movement_label(self.stats);
        let facing = format!(
            "{} · {:.1}° · {} · {:.1} m/s",
            heading_label(navigation.heading_degrees),
            normalized_heading(navigation.heading_degrees),
            pitch_label(navigation.pitch_degrees),
            navigation.horizontal_speed_metres_per_second,
        );
        if layout.compact {
            push_text(
                draw,
                format!("VOXEL {voxel_x} / {voxel_y} / {voxel_z}"),
                [layout.navigation.x + 11.0, layout.navigation.y + 59.0],
                9.0,
                TEXT_MUTED.with_alpha(opacity),
                TextAlign::Left,
            );
            push_text(
                draw,
                format!("CHUNK {chunk_x} / {chunk_y} / {chunk_z}"),
                [layout.navigation.x + 11.0, layout.navigation.y + 76.0],
                9.0,
                TEXT_MUTED.with_alpha(opacity),
                TextAlign::Left,
            );
            push_text(
                draw,
                format!("{facing} · {movement}"),
                [layout.navigation.x + 11.0, layout.navigation.y + 96.0],
                8.5,
                ACCENT.with_alpha(opacity),
                TextAlign::Left,
            );
        } else {
            push_text(
                draw,
                format!(
                    "VOXEL {voxel_x} / {voxel_y} / {voxel_z}   ·   CHUNK {chunk_x} / {chunk_y} / {chunk_z}"
                ),
                [layout.navigation.x + 11.0, layout.navigation.y + 59.0],
                9.5,
                TEXT_MUTED.with_alpha(opacity),
                TextAlign::Left,
            );
            push_text(
                draw,
                format!("FACING {facing} · {movement}"),
                [layout.navigation.x + 11.0, layout.navigation.y + 78.0],
                9.5,
                ACCENT.with_alpha(opacity),
                TextAlign::Left,
            );
        }
    }

    fn push_chrome(&self, draw: &mut UiDrawList, layout: &UiLayout) {
        // The wheel is gameplay chrome, while its selected item is already summarized in the
        // Mission Control header. Hiding it behind the modal prevents overlap on narrow screens.
        if !self.open {
            self.push_inventory_wheel(draw, layout.inventory);
        }

        let toast_alpha = if self.toast_age <= TOAST_HOLD_SECONDS {
            1.0
        } else {
            1.0 - ((self.toast_age - TOAST_HOLD_SECONDS) / TOAST_FADE_SECONDS).clamp(0.0, 1.0)
        };
        if toast_alpha > 0.001 {
            push_surface(
                draw,
                layout.toast,
                CHROME_HEIGHT * 0.5,
                PANEL_COLOR.with_alpha(toast_alpha),
                PANEL_BORDER.with_alpha(toast_alpha),
                SurfaceRole::Toast,
            );
            let default_toast = if self.stats.swimming {
                "WASD SWIM  /  SPACE ASCEND  /  SHIFT DIVE  /  F3 CONTROLS"
            } else if layout.toast.width < 500.0 {
                "WASD MOVE  /  SPACE JUMP  /  F3 CONTROLS"
            } else {
                "CLICK TO LOOK  /  WASD MOVE  /  SPACE JUMP  /  LMB DIG 0.5 M CUBE  /  RMB PLACE  /  F3"
            };
            let toast = self.gameplay_toast.as_deref().unwrap_or(default_toast);
            push_text(
                draw,
                toast,
                layout.toast.center(),
                9.5,
                TEXT_PRIMARY.with_alpha(toast_alpha),
                TextAlign::Center,
            );
        }

        let launcher_hover = self.hover_eased_value(UiTarget::Launcher);
        push_surface(
            draw,
            layout.launcher,
            CHROME_HEIGHT * 0.5,
            PANEL_COLOR.mix(HOVER_COLOR, launcher_hover),
            PANEL_BORDER.mix(ACCENT, launcher_hover * 0.6),
            SurfaceRole::Launcher,
        );
        let launcher = if self.stats.swimming {
            format!(
                "SWIMMING {:.0}% · {:.1} M   F3   {:.0} FPS",
                self.stats.water_immersion * 100.0,
                self.stats.eye_depth_metres,
                self.stats.frames_per_second
            )
        } else {
            format!(
                "MISSION CONTROL   F3   {:.0} FPS",
                self.stats.frames_per_second
            )
        };
        push_text(
            draw,
            launcher,
            layout.launcher.center(),
            10.5,
            TEXT_PRIMARY,
            TextAlign::Center,
        );

        push_surface(
            draw,
            layout.crosshair,
            6.0,
            Color::new(0.94, 0.98, 1.0, 0.88),
            Color::new(0.0, 0.0, 0.0, 0.56),
            SurfaceRole::Crosshair,
        );
    }

    fn push_inventory_wheel(&self, draw: &mut UiDrawList, rect: Rect) {
        let Some(selected) = self
            .selected_inventory_index
            .filter(|index| *index < self.inventory_items.len())
        else {
            let prompt = Rect::new(rect.center()[0] - 94.0, rect.y + 26.0, 188.0, 34.0);
            push_surface(
                draw,
                prompt,
                17.0,
                PANEL_COLOR,
                PANEL_BORDER,
                SurfaceRole::Inventory,
            );
            push_text(
                draw,
                "DIG TO COLLECT MATERIAL",
                prompt.center(),
                9.5,
                TEXT_MUTED,
                TextAlign::Center,
            );
            return;
        };

        let item_count = self.inventory_items.len();
        let center_x = rect.center()[0];
        let mut visible = Vec::<(i32, usize)>::new();
        for offset in [0_i32, -1, 1, -2, 2] {
            let index = (selected as i32 + offset).rem_euclid(item_count as i32) as usize;
            if !visible.iter().any(|(_, existing)| *existing == index) {
                visible.push((offset, index));
            }
            if visible.len() == item_count.min(5) {
                break;
            }
        }
        visible.sort_unstable_by_key(|(offset, _)| *offset);
        for (offset, index) in visible {
            let item = self.inventory_items[index];
            let distance = offset.unsigned_abs() as f32;
            let size = match offset.unsigned_abs() {
                0 => 50.0,
                1 => 38.0,
                _ => 30.0,
            };
            let x_offset = match offset {
                -2 => -116.0,
                -1 => -66.0,
                0 => 0.0,
                1 => 66.0,
                _ => 116.0,
            };
            let y_offset = match offset.unsigned_abs() {
                0 => 0.0,
                1 => 24.0,
                _ => 43.0,
            };
            let orb = Rect::new(
                center_x + x_offset - size * 0.5,
                rect.y + y_offset,
                size,
                size,
            );
            let selected_item = offset == 0;
            push_surface(
                draw,
                orb,
                size * 0.5,
                item.color
                    .with_alpha(if selected_item { 0.96 } else { 0.72 }),
                if selected_item {
                    ACCENT
                } else {
                    PANEL_BORDER.with_alpha((1.0 - distance * 0.2).max(0.35))
                },
                SurfaceRole::InventoryOrb,
            );
            push_text(
                draw,
                compact_count(item.count),
                [orb.center()[0], orb.y + orb.height + 8.0],
                if selected_item { 10.0 } else { 8.5 },
                if selected_item {
                    TEXT_PRIMARY
                } else {
                    TEXT_MUTED
                },
                TextAlign::Center,
            );
        }
        let selected_item = self.inventory_items[selected];
        push_text(
            draw,
            selected_item.label,
            [center_x, rect.y + rect.height - 7.0],
            9.5,
            ACCENT,
            TextAlign::Center,
        );
    }

    fn card_data(&self, compact: bool, card_count: usize) -> Vec<(&'static str, String)> {
        let stats = self.stats;
        if compact {
            vec![
                ("FRAME RATE", format!("{:.0} FPS", stats.frames_per_second)),
                ("FRAME", format!("{:.1} ms", stats.frame_ms)),
                (
                    "CHUNKS",
                    format!("{}/{}", stats.visible_chunks, stats.resident_chunks),
                ),
                (
                    "QUADS / WATER",
                    format!(
                        "{} / {}",
                        compact_count(u64::from(stats.quads)),
                        compact_count(u64::from(stats.water_quads))
                    ),
                ),
                ("PENDING JOBS", stats.pending_jobs.to_string()),
                ("GPU MEMORY", compact_bytes(stats.core_gpu_bytes)),
            ]
        } else {
            let mut cards = vec![
                (
                    "FRAME RATE",
                    format!(
                        "{:.0} FPS · {:.1} ms",
                        stats.frames_per_second, stats.frame_ms
                    ),
                ),
                (
                    "FRAME COST",
                    format!(
                        "CPU {:.1} · GPU {} · AO {}",
                        stats.cpu_ms,
                        optional_ms(stats.gpu_ms),
                        optional_ms(stats.gpu_ambient_occlusion_ms)
                    ),
                ),
                (
                    "CHUNKS",
                    format!(
                        "{} visible · {} resident",
                        stats.visible_chunks, stats.resident_chunks
                    ),
                ),
                (
                    "GEOMETRY",
                    format!(
                        "{} quads · {} draws",
                        compact_count(u64::from(stats.quads)),
                        stats.draw_calls,
                    ),
                ),
            ];
            if card_count > 6 {
                cards.push((
                    "LOD TILES 0 → 5",
                    stats
                        .lod_tiles
                        .map(u64::from)
                        .map(compact_count)
                        .join(" · "),
                ));
            }
            cards.extend([(
                "JOBS / EDITS",
                format!(
                    "{} pending · {} in flight",
                    stats.pending_jobs, stats.edit_in_flight,
                ),
            )]);
            if card_count > 6 {
                cards.push((
                    "LATENCY P95",
                    format!(
                        "load {} f · remesh {} f",
                        stats.load_p95_frames, stats.remesh_p95_frames,
                    ),
                ));
            }
            cards.push(("GPU MEMORY", compact_bytes(stats.core_gpu_bytes)));
            cards
        }
    }

    /// Produces the full text snapshot behind the visible summary cards. The browser shell can
    /// copy this string without teaching the portable renderer about clipboard APIs.
    pub fn diagnostics_report(&self) -> String {
        let stats = self.stats;
        let navigation = stats.navigation;
        let [x, y, z] = navigation.eye_position_metres;
        let [voxel_x, voxel_y, voxel_z] = navigation.eye_voxel;
        let [chunk_x, chunk_y, chunk_z] = navigation.eye_chunk;
        let mut report = String::from("VOXELS / MISSION CONTROL\n\n");

        let _ = writeln!(report, "NAVIGATION");
        let _ = writeln!(report, "Eye position (m): X {x:.3}, Y {y:.3}, Z {z:.3}");
        let _ = writeln!(
            report,
            "Eye voxel (10 cm): X {voxel_x}, Y {voxel_y}, Z {voxel_z}"
        );
        let _ = writeln!(report, "Chunk: X {chunk_x}, Y {chunk_y}, Z {chunk_z}");
        let _ = writeln!(
            report,
            "Facing: {} ({:.2} deg), pitch {}, speed {:.2} m/s, {}",
            heading_label(navigation.heading_degrees),
            normalized_heading(navigation.heading_degrees),
            pitch_label(navigation.pitch_degrees),
            navigation.horizontal_speed_metres_per_second,
            movement_label(stats).to_ascii_lowercase(),
        );

        let _ = writeln!(report, "\nWORLD");
        let _ = writeln!(
            report,
            "Time: {} ({})",
            self.world_time_label, self.daylight_label
        );
        let _ = writeln!(report, "Weather: {}", self.weather_label);
        let _ = writeln!(report, "Region: {}", self.region_label);
        let _ = writeln!(
            report,
            "Route: {} ({}%)",
            self.route_chapter_label, self.route_progress_percent,
        );
        if self.placement_material_available {
            let _ = writeln!(
                report,
                "Selected material: {} x{}",
                self.placement_material_label, self.placement_material_count,
            );
        } else {
            let _ = writeln!(report, "Selected material: none (inventory empty)");
        }

        let _ = writeln!(report, "\nPERFORMANCE");
        let _ = writeln!(
            report,
            "Frame: {:.1} FPS, {:.2} ms total, {:.2} ms CPU, {} ms GPU, {} ms GPU AO",
            stats.frames_per_second,
            stats.frame_ms,
            stats.cpu_ms,
            optional_ms(stats.gpu_ms),
            optional_ms(stats.gpu_ambient_occlusion_ms),
        );
        let _ = writeln!(
            report,
            "Core GPU memory: {}",
            compact_bytes(stats.core_gpu_bytes)
        );

        let _ = writeln!(report, "\nGEOMETRY");
        let _ = writeln!(
            report,
            "Chunks: {} visible, {} resident",
            stats.visible_chunks, stats.resident_chunks,
        );
        let _ = writeln!(
            report,
            "Quads: {} world, {} water",
            stats.quads, stats.water_quads,
        );
        let _ = writeln!(
            report,
            "Draw calls: {} world, {} water, {} shadow across {} cascades",
            stats.draw_calls,
            stats.water_draw_calls,
            stats.shadow_draw_calls,
            stats.shadow_cascades,
        );
        let _ = writeln!(
            report,
            "LOD tiles 0..5: {}",
            stats.lod_tiles.map(|count| count.to_string()).join(", "),
        );

        let _ = writeln!(report, "\nSTREAMING / EDITS");
        let _ = writeln!(report, "Pending jobs: {}", stats.pending_jobs);
        let _ = writeln!(
            report,
            "Load latency: p95 {} frames, max {} frames",
            stats.load_p95_frames, stats.load_max_frames,
        );
        let _ = writeln!(
            report,
            "Remesh latency: p95 {} frames, max {} frames",
            stats.remesh_p95_frames, stats.remesh_max_frames,
        );
        let _ = writeln!(
            report,
            "Edits: {} in flight, last {:.2} ms",
            stats.edit_in_flight, stats.edit_last_ms,
        );
        let _ = writeln!(
            report,
            "Interest: {} requested, {} desired, {} truncated, {} portal-active chunks",
            stats.stream_interest_requested,
            stats.stream_interest_desired,
            stats.stream_interest_truncated,
            stats.portal_active_chunks,
        );

        let _ = writeln!(report, "\nLIGHTS / PORTALS / WATER");
        let _ = writeln!(
            report,
            "Local lights: {} active / {} candidates, {} occluded, {} portal-rejected",
            stats.active_local_lights,
            stats.local_light_candidates,
            stats.occluded_local_lights,
            stats.portal_rejected_local_lights,
        );
        let _ = writeln!(
            report,
            "Cinder portals: {} open, revision {}",
            stats.open_cinder_portals, stats.cinder_portal_revision,
        );
        let _ = writeln!(
            report,
            "Water: {:.0}% immersed, eye depth {:.2} m, eyes submerged {}, swimming {}",
            stats.water_immersion * 100.0,
            stats.eye_depth_metres,
            stats.eyes_submerged,
            stats.swimming,
        );

        let _ = writeln!(report, "\nINVENTORY");
        if self.inventory_summary.iter().all(String::is_empty) {
            let _ = writeln!(report, "Empty");
        } else {
            for summary in &self.inventory_summary {
                if !summary.is_empty() {
                    let _ = writeln!(report, "{summary}");
                }
            }
        }

        let _ = writeln!(report, "\nRENDER FEATURES");
        for feature in RendererFeature::ALL {
            let state = if self.feature_enabled(feature) {
                "on"
            } else {
                "off"
            };
            let _ = writeln!(report, "{}: {state}", feature.label());
        }
        report
    }

    fn push_toggle(
        &self,
        draw: &mut UiDrawList,
        row: Rect,
        feature: RendererFeature,
        opacity: f32,
    ) {
        let value = self.feature_eased_value(feature).clamp(0.0, 1.0);
        let track = Rect::new(row.x + row.width - 42.0, row.center()[1] - 9.0, 34.0, 18.0);
        push_surface(
            draw,
            track,
            9.0,
            TOGGLE_OFF.mix(ACCENT, value).with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.55),
            SurfaceRole::ToggleTrack,
        );
        let thumb = Rect::new(track.x + 2.0 + value * 16.0, track.y + 2.0, 14.0, 14.0);
        push_surface(
            draw,
            thumb,
            7.0,
            TEXT_PRIMARY.with_alpha(opacity),
            Color::new(1.0, 1.0, 1.0, 0.22 * opacity),
            SurfaceRole::ToggleThumb,
        );
    }

    fn set_hover(&mut self, target: Option<UiTarget>) {
        if self.hovered == target {
            return;
        }
        if let Some(previous) = self.hovered {
            self.hover_motion
                .entry(previous)
                .or_insert(EasedValue::new(0.0))
                .set(false, self.reduced_motion);
        }
        self.hovered = target;
        if let Some(current) = target {
            self.hover_motion
                .entry(current)
                .or_insert(EasedValue::new(0.0))
                .set(true, self.reduced_motion);
        }
    }

    fn snap_motion(&mut self) {
        self.open_motion.advance(0.0, true);
        for motion in &mut self.feature_motion {
            motion.advance(0.0, true);
        }
        for motion in self.hover_motion.values_mut() {
            motion.advance(0.0, true);
        }
    }
}

fn push_surface(
    draw: &mut UiDrawList,
    rect: Rect,
    radius: f32,
    fill: Color,
    border: Color,
    role: SurfaceRole,
) {
    draw.glass.push(GlassSurface {
        rect,
        radius,
        fill,
        border,
        role,
    });
}

fn push_text(
    draw: &mut UiDrawList,
    text: impl Into<String>,
    position: [f32; 2],
    size: f32,
    color: Color,
    align: TextAlign,
) {
    draw.text.push(TextRun {
        text: text.into(),
        position,
        size,
        color,
        align,
    });
}

fn optional_ms(value: Option<f32>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.1}"))
}

fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn compact_bytes(value: u64) -> String {
    if value >= 1_073_741_824 {
        format!("{:.1} GiB", value as f64 / 1_073_741_824.0)
    } else if value >= 1_048_576 {
        format!("{:.1} MiB", value as f64 / 1_048_576.0)
    } else if value >= 1_024 {
        format!("{:.1} KiB", value as f64 / 1_024.0)
    } else {
        format!("{value} B")
    }
}

fn normalized_heading(heading_degrees: f32) -> f32 {
    if heading_degrees.is_finite() {
        heading_degrees.rem_euclid(360.0)
    } else {
        0.0
    }
}

fn heading_label(heading_degrees: f32) -> &'static str {
    const LABELS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let index = ((normalized_heading(heading_degrees) + 11.25) / 22.5).floor() as usize % 16;
    LABELS[index]
}

fn pitch_label(pitch_degrees: f32) -> String {
    if !pitch_degrees.is_finite() || pitch_degrees.abs() < 0.05 {
        "LEVEL".to_owned()
    } else if pitch_degrees > 0.0 {
        format!("{:.1}° UP", pitch_degrees.abs())
    } else {
        format!("{:.1}° DOWN", pitch_degrees.abs())
    }
}

fn movement_label(stats: LiveStats) -> &'static str {
    if stats.swimming {
        "SWIMMING"
    } else if stats.navigation.grounded {
        "GROUNDED"
    } else {
        "AIRBORNE"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Viewport {
        Viewport::new(1_280.0, 720.0, 1.0)
    }

    fn test_ui(config: MissionControlConfig, features: RendererFeatureConfig) -> MissionControlUi {
        MissionControlUi::new(config, features)
    }

    fn closed() -> MissionControlUi {
        test_ui(
            MissionControlConfig::default(),
            RendererFeatureConfig::default(),
        )
    }

    fn opened() -> MissionControlUi {
        test_ui(
            MissionControlConfig {
                open: true,
                compact: false,
            },
            RendererFeatureConfig::default(),
        )
    }

    #[test]
    fn f3_toggles_only_on_initial_key_down() {
        let mut ui = closed();
        assert_eq!(ui.handle_key(UiKey::Other, true, false), UiAction::None);
        assert_eq!(ui.handle_key(UiKey::F3, false, false), UiAction::None);
        assert_eq!(
            ui.handle_key(UiKey::F3, true, false),
            UiAction::PanelOpenChanged(true)
        );
        assert!(ui.open());
        assert_eq!(ui.handle_key(UiKey::F3, true, true), UiAction::None);
        assert!(ui.open());
        assert_eq!(
            ui.handle_key(UiKey::F3, true, false),
            UiAction::PanelOpenChanged(false)
        );
    }

    #[test]
    fn configured_panel_and_mixed_feature_state_are_applied_atomically() {
        let features = RendererFeatureConfig {
            cascaded_sun_shadows: false,
            voxel_ambient_occlusion: true,
            screen_space_ambient_occlusion: false,
            atmospheric_fog: true,
            far_terrain: false,
            water_surface: true,
            target_outline: false,
            material_surface_detail: true,
            cave_headlamp: false,
            voxel_emissive_lights: true,
        };
        let ui = test_ui(
            MissionControlConfig {
                open: true,
                compact: true,
            },
            features,
        );

        assert!(ui.open());
        assert!(ui.compact());
        for feature in RendererFeature::ALL {
            assert_eq!(ui.feature_enabled(feature), features.enabled(feature));
            assert_eq!(
                ui.feature_eased_value(feature),
                f32::from(features.enabled(feature))
            );
        }
    }

    #[test]
    fn device_pixel_hit_testing_matches_css_pixels_at_any_density() {
        let ui = opened();
        let normal = viewport();
        let retina = Viewport::new(2_560.0, 1_440.0, 2.0);
        let close = ui.layout(normal).region(UiTarget::Close);
        assert!(close.is_some());
        if let Some(close) = close {
            assert_eq!(
                ui.hit_test_css(close.center(), normal),
                Some(UiTarget::Close)
            );
            assert_eq!(
                ui.hit_test_device([close.center()[0] * 2.0, close.center()[1] * 2.0], retina),
                Some(UiTarget::Close)
            );
        }
    }

    #[test]
    fn copy_button_hit_testing_is_density_independent() {
        let normal = viewport();
        let retina = Viewport::new(2_560.0, 1_440.0, 2.0);
        let copy = opened()
            .layout(normal)
            .region(UiTarget::CopyDiagnostics)
            .expect("copy button");
        let mut normal_ui = opened();
        assert_eq!(
            normal_ui.activate_css(copy.center(), normal),
            UiAction::CopyDiagnostics,
        );
        let mut retina_ui = opened();
        assert_eq!(
            retina_ui.activate_device([copy.center()[0] * 2.0, copy.center()[1] * 2.0], retina,),
            UiAction::CopyDiagnostics,
        );
    }

    #[test]
    fn explicit_header_actions_are_hit_testable_without_a_secondary_menu() {
        let viewport = viewport();
        for (target, expected) in [(
            UiTarget::ResetRendererFeatures,
            UiAction::ResetRendererFeatures,
        )] {
            let mut ui = opened();
            let center = ui
                .layout(viewport)
                .region(target)
                .expect("header action")
                .center();
            assert_eq!(ui.activate_css(center, viewport), expected);
        }
    }

    #[test]
    fn desktop_layout_reserves_legible_navigation_and_two_column_cards() {
        let layout = opened().layout(viewport());
        assert_eq!(layout.panel.width, PANEL_WIDTH);
        assert!(layout.navigation.y >= layout.header.y + layout.header.height);
        assert_eq!(layout.stat_cards.len(), 8);
        assert!(layout.stat_cards.iter().all(|card| card.width >= 235.0));
        assert!(
            layout
                .stat_cards
                .iter()
                .all(|card| card.y >= layout.navigation.y + layout.navigation.height)
        );

        let buttons = [
            UiTarget::CopyDiagnostics,
            UiTarget::ResetRendererFeatures,
            UiTarget::Compact,
            UiTarget::Close,
        ]
        .map(|target| layout.region(target).expect("header action"));
        for (index, left) in buttons.iter().enumerate() {
            for right in &buttons[index + 1..] {
                assert!(left.x + left.width <= right.x || right.x + right.width <= left.x);
            }
        }
    }

    #[test]
    fn heading_labels_wrap_and_change_at_half_sector_boundaries() {
        assert_eq!(heading_label(0.0), "N");
        assert_eq!(heading_label(11.24), "N");
        assert_eq!(heading_label(11.25), "NNE");
        assert_eq!(heading_label(90.0), "E");
        assert_eq!(heading_label(118.9), "ESE");
        assert_eq!(heading_label(-90.0), "W");
        assert_eq!(heading_label(359.99), "N");
        assert_eq!(heading_label(720.0), "N");
    }

    #[test]
    fn diagnostics_report_includes_full_navigation_and_hidden_details() {
        let mut ui = opened();
        let _ = ui.set_feature(RendererFeature::FarTerrain, false);
        ui.set_inventory(
            Some("STONE"),
            12,
            ["STONE 12 · GRASS 4".to_owned(), String::new()],
            Vec::new(),
            None,
        );
        ui.set_stats(LiveStats {
            navigation: NavigationTelemetry {
                eye_position_metres: [-0.01, 12.345, -3.201],
                eye_voxel: [-1, 123, -33],
                eye_chunk: [-1, 3, -2],
                heading_degrees: 270.0,
                pitch_degrees: 12.5,
                horizontal_speed_metres_per_second: 5.75,
                grounded: false,
            },
            frames_per_second: 120.0,
            frame_ms: 8.33,
            cpu_ms: 1.25,
            gpu_ms: Some(4.5),
            lod_tiles: [75, 16, 8, 4, 2, 1],
            shadow_draw_calls: 42,
            shadow_cascades: 3,
            stream_interest_requested: 80,
            stream_interest_desired: 72,
            stream_interest_truncated: 8,
            ..LiveStats::default()
        });

        let report = ui.diagnostics_report();
        assert!(report.contains("Eye position (m): X -0.010, Y 12.345, Z -3.201"));
        assert!(report.contains("Eye voxel (10 cm): X -1, Y 123, Z -33"));
        assert!(report.contains("Chunk: X -1, Y 3, Z -2"));
        assert!(report.contains("Facing: W (270.00 deg), pitch 12.5° UP"));
        assert!(report.contains("LOD tiles 0..5: 75, 16, 8, 4, 2, 1"));
        assert!(report.contains("42 shadow across 3 cascades"));
        assert!(report.contains("80 requested, 72 desired, 8 truncated"));
        assert!(report.contains("STONE 12 · GRASS 4"));
        assert!(report.contains("Far terrain: off"));
    }

    #[test]
    fn feature_rows_toggle_and_animate_without_changing_host_state_implicitly() {
        let mut ui = opened();
        let feature = RendererFeature::AtmosphericFog;
        assert!(ui.feature_enabled(feature));
        assert_eq!(
            ui.toggle_feature(feature),
            UiAction::FeatureChanged(feature, false)
        );
        assert!(!ui.feature_enabled(feature));
        assert_eq!(ui.feature_eased_value(feature), 1.0);
        ui.advance(1.0 / 60.0);
        assert!(ui.feature_eased_value(feature) > 0.0);
        assert!(ui.feature_eased_value(feature) < 1.0);
        for _ in 0..60 {
            ui.advance(1.0 / 60.0);
        }
        assert_eq!(ui.feature_eased_value(feature), 0.0);
    }

    #[test]
    fn hover_fades_between_shape_matched_regions() {
        let mut ui = opened();
        let ambient_occlusion = UiTarget::Feature(RendererFeature::VoxelAmbientOcclusion);
        let fog = UiTarget::Feature(RendererFeature::AtmosphericFog);
        let layout = ui.layout(viewport());
        let ambient_occlusion_center = layout.region(ambient_occlusion).map(Rect::center);
        let fog_center = layout.region(fog).map(Rect::center);
        assert!(ambient_occlusion_center.is_some() && fog_center.is_some());
        if let (Some(ambient_occlusion_center), Some(fog_center)) =
            (ambient_occlusion_center, fog_center)
        {
            assert!(ui.pointer_move_device(ambient_occlusion_center, viewport()));
            ui.advance(1.0 / 60.0);
            let entered = ui.hover_eased_value(ambient_occlusion);
            assert!(entered > 0.0 && entered < 1.0);
            assert!(ui.pointer_move_device(fog_center, viewport()));
            ui.advance(1.0 / 60.0);
            assert!(ui.hover_eased_value(ambient_occlusion) < entered);
            assert!(ui.hover_eased_value(fog) > 0.0);
        }
    }

    #[test]
    fn reduced_motion_snaps_open_hover_and_toggle_values() {
        let mut ui = closed();
        ui.set_reduced_motion(true);
        let _ = ui.set_open(true);
        assert_eq!(ui.open_motion.value, 1.0);
        let ambient_occlusion = RendererFeature::VoxelAmbientOcclusion;
        let _ = ui.set_feature(ambient_occlusion, false);
        assert_eq!(ui.feature_eased_value(ambient_occlusion), 0.0);
        let target = UiTarget::Feature(ambient_occlusion);
        let center = ui.layout(viewport()).region(target).map(Rect::center);
        if let Some(center) = center {
            let _ = ui.pointer_move_device(center, viewport());
            assert_eq!(ui.hover_eased_value(target), 1.0);
        }
    }

    #[test]
    fn compact_mode_reflows_cards_and_is_automatic_on_narrow_viewports() {
        let mut ui = opened();
        let normal = ui.layout(viewport());
        assert!(!normal.compact);
        assert_eq!(normal.stat_cards.len(), 8);
        let _ = ui.set_compact(true);
        let compact = ui.layout(viewport());
        assert!(compact.compact);
        assert_eq!(compact.stat_cards.len(), 6);
        assert!(compact.panel.width < normal.panel.width);

        let automatic = opened();
        let narrow_viewport = Viewport::new(640.0, 720.0, 1.0);
        let narrow = automatic.layout(narrow_viewport);
        assert!(narrow.compact);
        assert!(!automatic.compact());
        assert_eq!(narrow.region(UiTarget::Compact), None);
        assert!(
            !automatic
                .build_draw_list(narrow_viewport)
                .text
                .iter()
                .any(|run| run.text == ">")
        );
    }

    #[test]
    fn narrow_viewport_keeps_launcher_and_panel_inside_the_viewport() {
        let viewport = Viewport::new(390.0, 844.0, 1.0);
        let layout = opened().layout(viewport);

        assert!(layout.launcher.y + layout.launcher.height < layout.panel.y);
        for rect in [
            layout.launcher,
            layout.inventory,
            layout.panel,
            layout.header,
            layout.navigation,
        ] {
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.width <= viewport.css_size()[0]);
            assert!(rect.y + rect.height <= viewport.css_size()[1]);
        }
        assert!(layout.stat_cards.iter().all(|rect| {
            rect.x >= layout.panel.x
                && rect.y >= layout.panel.y
                && rect.x + rect.width <= layout.panel.x + layout.panel.width
                && rect.y + rect.height <= layout.panel.y + layout.panel.height
        }));
    }

    #[test]
    fn every_feature_row_stays_inside_the_glass_panel() {
        let ui = opened();
        for viewport in [
            Viewport::new(1_280.0, 720.0, 1.0),
            Viewport::new(960.0, 640.0, 1.0),
            Viewport::new(640.0, 720.0, 1.0),
        ] {
            let layout = ui.layout(viewport);
            for feature in RendererFeature::ALL {
                let row = layout.region(UiTarget::Feature(feature));
                assert!(row.is_some());
                if let Some(row) = row {
                    assert!(row.x >= layout.panel.x);
                    assert!(row.y >= layout.panel.y);
                    assert!(row.x + row.width <= layout.panel.x + layout.panel.width);
                    assert!(row.y + row.height <= layout.panel.y + layout.panel.height);
                    assert!(row.height >= 24.0);
                }
            }
        }
    }

    #[test]
    fn draw_list_has_reusable_glass_text_cards_and_toggles() {
        let mut ui = opened();
        ui.set_reduced_motion(true);
        ui.set_route_status("WINDCUT WAY", 47);
        ui.set_inventory(
            Some("GLOW CRYSTAL"),
            27,
            [
                "GR 4 · DI 8 · ST 12 · SA 0 · SN 0 · CL 3 · BA 2".to_owned(),
                "WO 9 · LE 6 · MO 1 · LI 5 · RS 0 · WA 2 · GL 27".to_owned(),
            ],
            vec![
                InventoryItem {
                    label: "STONE",
                    count: 12,
                    color: Color::new(0.34, 0.38, 0.43, 0.92),
                },
                InventoryItem {
                    label: "GLOW CRYSTAL",
                    count: 27,
                    color: Color::new(0.12, 0.58, 0.78, 0.92),
                },
            ],
            Some(1),
        );
        ui.set_stats(LiveStats {
            navigation: NavigationTelemetry {
                eye_position_metres: [420.845, 153.34, -608.25],
                eye_voxel: [4_208, 1_533, -6_083],
                eye_chunk: [131, 47, -191],
                heading_degrees: 118.9,
                pitch_degrees: -21.3,
                horizontal_speed_metres_per_second: 4.25,
                grounded: true,
            },
            frames_per_second: 59.8,
            frame_ms: 16.7,
            cpu_ms: 4.2,
            gpu_ms: Some(7.8),
            gpu_ambient_occlusion_ms: Some(1.2),
            resident_chunks: 481,
            visible_chunks: 132,
            quads: 123_400,
            water_quads: 8_200,
            draw_calls: 132,
            water_draw_calls: 12,
            shadow_draw_calls: 164,
            shadow_cascades: 3,
            load_p95_frames: 18,
            load_max_frames: 31,
            remesh_p95_frames: 2,
            remesh_max_frames: 4,
            edit_last_ms: 31.2,
            edit_in_flight: 1,
            lod_tiles: [49, 49, 49, 81, 49, 81],
            pending_jobs: 7,
            core_gpu_bytes: 76 * 1_048_576,
            water_immersion: 0.0,
            eye_depth_metres: 0.0,
            eyes_submerged: false,
            swimming: false,
            local_light_candidates: 10,
            active_local_lights: 6,
            occluded_local_lights: 1,
            portal_rejected_local_lights: 3,
            open_cinder_portals: 6,
            cinder_portal_revision: 2,
            stream_interest_requested: 80,
            stream_interest_desired: 72,
            stream_interest_truncated: 8,
            portal_active_chunks: 68,
        });
        let base = ui.build_draw_list(viewport());
        assert!(base.text.iter().any(|run| run.text == "60 FPS · 16.7 ms"));
        assert!(
            base.text
                .iter()
                .any(|run| run.text == "123.4k quads · 132 draws")
        );
        assert!(
            base.text
                .iter()
                .any(|run| run.text == "7 pending · 1 in flight")
        );
        assert!(base.text.iter().any(|run| run.text == "76.0 MiB"));
        assert!(
            base.text
                .iter()
                .any(|run| run.text == "X 420.85   Y 153.34   Z -608.25 m")
        );
        assert!(
            base.text.iter().any(|run| {
                run.text == "VOXEL 4208 / 1533 / -6083   ·   CHUNK 131 / 47 / -191"
            })
        );
        assert!(base.text.iter().any(|run| {
            run.text == "FACING ESE · 118.9° · 21.3° DOWN · 4.2 m/s · GROUNDED"
        }));
        assert!(
            base.text
                .iter()
                .any(|run| run.text == "17:17 · GOLDEN HOUR / VERDANT FOREST")
        );
        assert!(
            base.text
                .iter()
                .any(|run| run.text == "PLACE GLOW CRYSTAL ×27")
        );
        let _ = ui.set_open(false);
        let inventory_draw = ui.build_draw_list(viewport());
        assert!(
            inventory_draw
                .text
                .iter()
                .any(|run| run.text == "GLOW CRYSTAL")
        );
        assert!(inventory_draw.text.iter().any(|run| run.text == "27"));
        let inventory_orbs = inventory_draw
            .glass
            .iter()
            .filter(|surface| surface.role == SurfaceRole::InventoryOrb)
            .collect::<Vec<_>>();
        assert_eq!(inventory_orbs.len(), 2);
        assert!(
            inventory_orbs
                .iter()
                .any(|surface| surface.rect.width == 50.0)
        );

        let _ = ui.set_open(true);
        let draw = ui.build_draw_list(viewport());
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::Panel)
        );
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::StatCard)
                .count(),
            8
        );
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::ToggleTrack)
                .count(),
            RendererFeature::ALL.len()
        );
        assert!(draw.text.iter().any(|run| run.text == "COPY"));
        assert!(draw.text.iter().any(|run| run.text == "RESET"));
        assert!(!draw.text.iter().any(|run| run.text == "CPU / GPU"));
    }

    #[test]
    fn closed_panel_keeps_chrome_but_removes_panel_after_easing_out() {
        let mut ui = opened();
        ui.advance(1.0);
        let _ = ui.set_open(false);
        assert_eq!(ui.hit_test_css([1_000.0, 30.0], viewport()), None);
        for _ in 0..60 {
            ui.advance(1.0 / 60.0);
        }
        let draw = ui.build_draw_list(viewport());
        assert!(
            !draw
                .glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::Panel)
        );
        assert!(
            !draw
                .text
                .iter()
                .any(|run| run.text == "VOXELS / 10 CM CUBES")
        );
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::Launcher)
                .count(),
            1
        );
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::Inventory)
                .count(),
            1
        );
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::Crosshair)
                .count(),
            1
        );
    }

    #[test]
    fn launcher_is_always_hit_testable_and_opens_the_panel() {
        let mut ui = closed();
        let viewport = viewport();
        let launcher = ui.layout(viewport).launcher;
        assert_eq!(
            ui.hit_test_css(launcher.center(), viewport),
            Some(UiTarget::Launcher)
        );
        assert_eq!(
            ui.activate_css(launcher.center(), viewport),
            UiAction::PanelOpenChanged(true)
        );
        assert!(ui.open());
        let draw = ui.build_draw_list(viewport);
        assert!(
            !draw
                .text
                .iter()
                .any(|run| run.text == "VOXELS / 10 CM CUBES")
        );
        assert!(
            draw.text
                .iter()
                .any(|run| run.text.contains("MISSION CONTROL   F3"))
        );
    }

    #[test]
    fn invalid_scale_factor_falls_back_to_one() {
        let viewport = Viewport::new(800.0, 600.0, 0.0);
        assert_eq!(viewport.scale_factor, 1.0);
        assert_eq!(viewport.device_to_css([20.0, 30.0]), [20.0, 30.0]);
    }

    #[test]
    fn controls_toast_is_rust_drawn_then_fades_away() {
        let mut ui = closed();
        let initial = ui.build_draw_list(viewport());
        assert!(
            initial
                .glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::Toast)
        );
        for _ in 0..500 {
            ui.advance(1.0 / 60.0);
        }
        let settled = ui.build_draw_list(viewport());
        assert!(
            settled
                .glass
                .iter()
                .all(|surface| surface.role != SurfaceRole::Toast)
        );
    }

    #[test]
    fn gameplay_toast_replaces_controls_and_uses_the_same_bounded_lifetime() {
        let mut ui = closed();
        for _ in 0..500 {
            ui.advance(1.0 / 60.0);
        }
        ui.show_gameplay_toast("Too far away to dig");
        let visible = ui.build_draw_list(viewport());
        assert!(
            visible
                .text
                .iter()
                .any(|run| run.text == "Too far away to dig")
        );
        for _ in 0..500 {
            ui.advance(1.0 / 60.0);
        }
        assert!(
            ui.build_draw_list(viewport())
                .glass
                .iter()
                .all(|surface| surface.role != SurfaceRole::Toast)
        );
    }

    #[test]
    fn crosshair_is_a_rust_drawn_circle() {
        let ui = closed();
        let draw = ui.build_draw_list(viewport());
        let crosshair = draw
            .glass
            .iter()
            .find(|surface| surface.role == SurfaceRole::Crosshair)
            .expect("Rust UI must always emit the crosshair");
        assert_eq!(crosshair.rect.width, crosshair.rect.height);
        assert_eq!(crosshair.radius, crosshair.rect.width * 0.5);
        assert!(crosshair.fill.0[3] > 0.0);
    }

    #[test]
    fn underwater_status_and_controls_are_rust_drawn() {
        let mut ui = closed();
        for _ in 0..500 {
            ui.advance(1.0 / 60.0);
        }
        assert!(
            ui.build_draw_list(viewport())
                .glass
                .iter()
                .all(|surface| surface.role != SurfaceRole::Toast)
        );
        ui.set_stats(LiveStats {
            frames_per_second: 60.0,
            water_immersion: 0.82,
            eye_depth_metres: 0.7,
            eyes_submerged: true,
            swimming: true,
            ..LiveStats::default()
        });
        let draw = ui.build_draw_list(viewport());
        assert!(
            draw.text
                .iter()
                .any(|run| run.text.contains("SPACE ASCEND") && run.text.contains("SHIFT DIVE"))
        );
        assert!(
            draw.text
                .iter()
                .any(|run| run.text.contains("SWIMMING 82% · 0.7 M"))
        );
    }
}
