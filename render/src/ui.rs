//! Portable state, layout, hit-testing, and draw-list generation for the in-canvas mission control.
//!
//! Coordinates in this module are CSS/device-independent pixels. The browser shell may forward raw
//! device pixels through [`Viewport::device_to_css`], but the state and resulting draw list contain
//! no browser, WASM, WGPU, or resource-lifetime concerns.

use crate::environment::{DaylightPhase, WeatherPreset};
use std::{collections::BTreeMap, fmt::Write};
use voxels_world::protocol::EditShape;

const PANEL_INSET: f32 = 18.0;
const PANEL_WIDTH: f32 = 500.0;
const COMPACT_WIDTH: f32 = 340.0;
const HEADER_HEIGHT: f32 = 76.0;
const NAVIGATION_HEIGHT: f32 = 92.0;
const COMPACT_NAVIGATION_HEIGHT: f32 = 108.0;
const PANEL_RADIUS: f32 = 24.0;
const CONTENT_PAD: f32 = 16.0;
const BUTTON_SIZE: f32 = 40.0;
const BUTTON_GAP: f32 = 8.0;
const ACTION_BUTTON_WIDTH: f32 = 86.0;
const CHROME_HEIGHT: f32 = 44.0;
const LAUNCHER_WIDTH: f32 = 246.0;
const INVENTORY_WIDTH: f32 = 430.0;
const INVENTORY_HEIGHT: f32 = 188.0;
const EDIT_SHAPE_WIDTH: f32 = 154.0;
const EDIT_SHAPE_COMPACT_WIDTH: f32 = 82.0;
const EDIT_SHAPE_HEIGHT: f32 = 66.0;
const CHROME_STACK_GAP: f32 = 8.0;
const PANEL_TOP: f32 = PANEL_INSET + CHROME_HEIGHT + 12.0;
const AUTO_COMPACT_WIDTH: f32 = 720.0;
const MOTION_RATE: f32 = 18.0;
const TOAST_HOLD_SECONDS: f32 = 5.0;
const TOAST_FADE_SECONDS: f32 = 1.5;

const PANEL_COLOR: Color = Color::new(0.036, 0.043, 0.064, 0.91);
const PANEL_BORDER: Color = Color::new(0.88, 0.91, 1.0, 0.18);
const CARD_COLOR: Color = Color::new(0.072, 0.078, 0.105, 0.78);
const HOVER_COLOR: Color = Color::new(0.98, 0.77, 0.29, 0.20);
const TEXT_PRIMARY: Color = Color::new(0.975, 0.965, 0.93, 1.0);
const TEXT_MUTED: Color = Color::new(0.66, 0.68, 0.74, 1.0);
const ACCENT: Color = Color::new(0.98, 0.78, 0.31, 1.0);
const SUCCESS: Color = Color::new(0.42, 0.88, 0.63, 1.0);
const TOGGLE_OFF: Color = Color::new(0.13, 0.14, 0.18, 0.96);

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

/// Configured renderer feature baseline. Mission Control intentionally does not expose these as
/// casual toggles; stable visual features belong in configuration, while the panel focuses on
/// playful world controls and useful live information.
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
    pub developer_controls: bool,
    pub spectator_available: bool,
}

const PANEL_HEIGHT: f32 = 632.0;
const COMPACT_HEIGHT: f32 = 650.0;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum TimeControl {
    #[default]
    FollowServer,
    Dawn,
    Noon,
    GoldenHour,
    BlueHour,
    Night,
}

impl TimeControl {
    pub const ALL: [Self; 6] = [
        Self::FollowServer,
        Self::Dawn,
        Self::Noon,
        Self::GoldenHour,
        Self::BlueHour,
        Self::Night,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::FollowServer => "LIVE",
            Self::Dawn => "DAWN",
            Self::Noon => "NOON",
            Self::GoldenHour => "GOLDEN",
            Self::BlueHour => "BLUE",
            Self::Night => "NIGHT",
        }
    }

    pub const fn day_fraction(self) -> Option<f32> {
        match self {
            Self::FollowServer => None,
            Self::Dawn => Some(DaylightPhase::Dawn.anchor_day_fraction()),
            Self::Noon => Some(DaylightPhase::ClearDay.anchor_day_fraction()),
            Self::GoldenHour => Some(DaylightPhase::GoldenHour.anchor_day_fraction()),
            Self::BlueHour => Some(DaylightPhase::BlueHour.anchor_day_fraction()),
            Self::Night => Some(DaylightPhase::Night.anchor_day_fraction()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum WeatherControl {
    #[default]
    FollowServer,
    Clear,
    Cloudy,
    Overcast,
    Rain,
    Storm,
}

impl WeatherControl {
    pub const ALL: [Self; 6] = [
        Self::FollowServer,
        Self::Clear,
        Self::Cloudy,
        Self::Overcast,
        Self::Rain,
        Self::Storm,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::FollowServer => "LIVE",
            Self::Clear => "CLEAR",
            Self::Cloudy => "CLOUDY",
            Self::Overcast => "OVERCAST",
            Self::Rain => "RAIN",
            Self::Storm => "STORM",
        }
    }

    pub const fn preset(self) -> Option<WeatherPreset> {
        match self {
            Self::FollowServer => None,
            Self::Clear => Some(WeatherPreset::Clear),
            Self::Cloudy => Some(WeatherPreset::Cloudy),
            Self::Overcast => Some(WeatherPreset::Overcast),
            Self::Rain => Some(WeatherPreset::Rain),
            Self::Storm => Some(WeatherPreset::Storm),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum UiTarget {
    Launcher,
    EditShape,
    Header,
    CopyDiagnostics,
    DiagnosticSky,
    TakeScreenshot,
    Close,
    Time(TimeControl),
    Weather(WeatherControl),
    Spectator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiKey {
    F3,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    None,
    EditShapeChanged(EditShape),
    CopyDiagnostics,
    DiagnosticSkyChanged(bool),
    TakeScreenshot,
    PanelOpenChanged(bool),
    TimeChanged(TimeControl),
    WeatherChanged(WeatherControl),
    SpectatorRequested(bool),
}

trait SegmentValue<T> {
    fn segment_value(self) -> T;
    fn segment_label(self) -> &'static str;
}

impl SegmentValue<TimeControl> for UiTarget {
    fn segment_value(self) -> TimeControl {
        match self {
            Self::Time(value) => value,
            _ => TimeControl::FollowServer,
        }
    }

    fn segment_label(self) -> &'static str {
        <Self as SegmentValue<TimeControl>>::segment_value(self).label()
    }
}

impl SegmentValue<WeatherControl> for UiTarget {
    fn segment_value(self) -> WeatherControl {
        match self {
            Self::Weather(value) => value,
            _ => WeatherControl::FollowServer,
        }
    }

    fn segment_label(self) -> &'static str {
        <Self as SegmentValue<WeatherControl>>::segment_value(self).label()
    }
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
    pub spectator: bool,
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
    WorldCard,
    Segment,
    AuthorityBadge,
    MovementCard,
    DiagnosticsCard,
    NavigationCard,
    Button,
    StatCard,
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
    pub edit_shape: Rect,
    pub toast: Rect,
    pub crosshair: Rect,
    pub panel: Rect,
    pub header: Rect,
    pub world_card: Rect,
    pub movement_card: Rect,
    pub diagnostics_card: Rect,
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

/// Owns presentation and transient local selections. Semantic actions are emitted to the renderer
/// or shell and confirmed state is fed back explicitly.
pub struct MissionControlUi {
    open: bool,
    developer_controls: bool,
    spectator_available: bool,
    spectator_active: bool,
    diagnostic_sky_active: bool,
    time_control: TimeControl,
    weather_control: WeatherControl,
    reduced_motion: bool,
    hovered: Option<UiTarget>,
    stats: LiveStats,
    open_motion: EasedValue,
    spectator_motion: EasedValue,
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
    edit_shape: EditShape,
}

impl Default for MissionControlUi {
    fn default() -> Self {
        Self::new(MissionControlConfig::default())
    }
}

impl MissionControlUi {
    pub fn new(config: MissionControlConfig) -> Self {
        Self {
            open: config.open,
            developer_controls: config.developer_controls,
            spectator_available: config.spectator_available,
            spectator_active: false,
            diagnostic_sky_active: false,
            time_control: TimeControl::FollowServer,
            weather_control: WeatherControl::FollowServer,
            reduced_motion: false,
            hovered: None,
            stats: LiveStats::default(),
            open_motion: EasedValue::new(f32::from(config.open)),
            spectator_motion: EasedValue::new(0.0),
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
            edit_shape: EditShape::Sphere,
        }
    }

    pub const fn open(&self) -> bool {
        self.open
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

    pub fn set_spectator_active(&mut self, active: bool) {
        let active = active && self.spectator_available;
        self.spectator_active = active;
        self.spectator_motion.set(active, self.reduced_motion);
    }

    pub fn set_spectator_available(&mut self, available: bool) {
        self.spectator_available = available;
        if !available {
            self.set_spectator_active(false);
        }
    }

    pub const fn diagnostic_sky_active(&self) -> bool {
        self.diagnostic_sky_active
    }

    pub fn set_diagnostic_sky_active(&mut self, active: bool) {
        self.diagnostic_sky_active = active;
    }

    pub const fn time_control(&self) -> TimeControl {
        self.time_control
    }

    pub const fn weather_control(&self) -> WeatherControl {
        self.weather_control
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

    pub const fn edit_shape(&self) -> EditShape {
        self.edit_shape
    }

    pub fn set_edit_shape(&mut self, shape: EditShape) {
        self.edit_shape = shape;
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

    pub fn hover_eased_value(&self, target: UiTarget) -> f32 {
        self.hover_motion
            .get(&target)
            .map_or(0.0, |motion| motion.value)
    }

    pub fn advance(&mut self, dt: f32) {
        self.toast_age += dt.clamp(0.0, 0.1);
        self.open_motion.advance(dt, self.reduced_motion);
        self.spectator_motion.advance(dt, self.reduced_motion);
        for motion in self.hover_motion.values_mut() {
            motion.advance(dt, self.reduced_motion);
        }
    }

    pub fn effective_compact(&self, viewport: Viewport) -> bool {
        viewport.css_size()[0] < AUTO_COMPACT_WIDTH
    }

    pub fn layout(&self, viewport: Viewport) -> UiLayout {
        let [viewport_width, viewport_height] = viewport.css_size();
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
        let edit_shape_width = if compact {
            EDIT_SHAPE_COMPACT_WIDTH
        } else {
            EDIT_SHAPE_WIDTH
        };
        let edit_shape = Rect::new(
            PANEL_INSET.min((viewport_width - edit_shape_width).max(0.0)),
            if compact {
                (inventory.y - CHROME_STACK_GAP - EDIT_SHAPE_HEIGHT).max(0.0)
            } else {
                (viewport_height - PANEL_INSET - EDIT_SHAPE_HEIGHT).max(0.0)
            },
            edit_shape_width.min(viewport_width.max(0.0)),
            EDIT_SHAPE_HEIGHT,
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
                target: UiTarget::EditShape,
                rect: edit_shape,
            },
            InteractiveRegion {
                target: UiTarget::Header,
                rect: header,
            },
        ];

        let button_y = header.y + 12.0;
        let close = Rect::new(
            panel.x + panel.width - CONTENT_PAD - BUTTON_SIZE,
            button_y,
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        let copy = Rect::new(
            close.x - BUTTON_GAP - ACTION_BUTTON_WIDTH,
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
                target: UiTarget::Close,
                rect: close,
            },
        ]);

        let world_card = Rect::new(
            panel.x + CONTENT_PAD,
            panel.y + HEADER_HEIGHT + 10.0,
            panel.width - CONTENT_PAD * 2.0,
            if compact { 184.0 } else { 154.0 },
        );
        let chip_columns = if compact { 3 } else { 6 };
        let chip_gap = 6.0;
        let chip_width =
            (world_card.width - 20.0 - chip_gap * (chip_columns - 1) as f32) / chip_columns as f32;
        let chip_height = if compact { 28.0 } else { 34.0 };
        let time_top = world_card.y + 30.0;
        for (index, control) in TimeControl::ALL.into_iter().enumerate() {
            let column = index % chip_columns;
            let row = index / chip_columns;
            regions.push(InteractiveRegion {
                target: UiTarget::Time(control),
                rect: Rect::new(
                    world_card.x + 10.0 + column as f32 * (chip_width + chip_gap),
                    time_top + row as f32 * (chip_height + 5.0),
                    chip_width,
                    chip_height,
                ),
            });
        }
        let time_rows = TimeControl::ALL.len().div_ceil(chip_columns);
        let weather_top = time_top + time_rows as f32 * (chip_height + 5.0) + 24.0;
        for (index, control) in WeatherControl::ALL.into_iter().enumerate() {
            let column = index % chip_columns;
            let row = index / chip_columns;
            regions.push(InteractiveRegion {
                target: UiTarget::Weather(control),
                rect: Rect::new(
                    world_card.x + 10.0 + column as f32 * (chip_width + chip_gap),
                    weather_top + row as f32 * (chip_height + 5.0),
                    chip_width,
                    chip_height,
                ),
            });
        }

        let movement_card = Rect::new(
            world_card.x,
            world_card.y + world_card.height + 10.0,
            world_card.width,
            60.0,
        );
        if self.developer_controls && self.spectator_available {
            regions.push(InteractiveRegion {
                target: UiTarget::Spectator,
                rect: movement_card,
            });
        }
        let diagnostics_card = Rect::new(
            world_card.x,
            movement_card.y + movement_card.height + 10.0,
            world_card.width,
            70.0,
        );
        let screenshot_width = if compact { 104.0 } else { 128.0 };
        let screenshot = Rect::new(
            diagnostics_card.x + diagnostics_card.width - 10.0 - screenshot_width,
            diagnostics_card.y + 30.0,
            screenshot_width,
            40.0,
        );
        let diagnostic_sky = Rect::new(
            diagnostics_card.x + 10.0,
            diagnostics_card.y + 27.0,
            (screenshot.x - diagnostics_card.x - 20.0).max(40.0),
            46.0,
        );
        regions.extend([
            InteractiveRegion {
                target: UiTarget::DiagnosticSky,
                rect: diagnostic_sky,
            },
            InteractiveRegion {
                target: UiTarget::TakeScreenshot,
                rect: screenshot,
            },
        ]);
        let navigation = Rect::new(
            world_card.x,
            diagnostics_card.y + diagnostics_card.height + 10.0,
            world_card.width,
            if compact {
                COMPACT_NAVIGATION_HEIGHT
            } else {
                NAVIGATION_HEIGHT
            },
        );
        let stats_top = navigation.y + navigation.height + 16.0;
        let stat_columns: usize = 2;
        let stat_gap = 8.0;
        let stat_width = (panel.width - CONTENT_PAD * 2.0 - stat_gap * (stat_columns - 1) as f32)
            / stat_columns as f32;
        let stat_height = 48.0;
        let available_stats = (panel.y + panel.height - CONTENT_PAD - stats_top).max(0.0);
        let stat_count = if available_stats >= stat_height * 2.0 + stat_gap {
            4
        } else if available_stats >= stat_height {
            2
        } else {
            0
        };
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

        UiLayout {
            launcher,
            inventory,
            edit_shape,
            toast,
            crosshair,
            panel,
            header,
            world_card,
            movement_card,
            diagnostics_card,
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
                .find(|region| {
                    matches!(region.target, UiTarget::Launcher | UiTarget::EditShape)
                        && region.rect.contains(point)
                        && (region.target != UiTarget::EditShape || !self.spectator_active)
                })
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
        !self.spectator_active && self.layout(viewport).inventory.contains(point)
    }

    pub fn edit_shape_contains_css(&self, point: [f32; 2], viewport: Viewport) -> bool {
        !self.open && !self.spectator_active && self.layout(viewport).edit_shape.contains(point)
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
        match self.hit_test_css(point, viewport) {
            Some(UiTarget::Launcher) => return self.toggle_open(),
            Some(UiTarget::EditShape) if !self.open && !self.spectator_active => {
                self.edit_shape = self.edit_shape.next();
                return UiAction::EditShapeChanged(self.edit_shape);
            }
            _ => {}
        }
        if !self.open {
            return UiAction::None;
        }
        match self.hit_test_css(point, viewport) {
            Some(UiTarget::Launcher) => self.toggle_open(),
            Some(UiTarget::EditShape) => UiAction::None,
            Some(UiTarget::CopyDiagnostics) => UiAction::CopyDiagnostics,
            Some(UiTarget::DiagnosticSky) if self.developer_controls => {
                self.diagnostic_sky_active = !self.diagnostic_sky_active;
                UiAction::DiagnosticSkyChanged(self.diagnostic_sky_active)
            }
            Some(UiTarget::TakeScreenshot) => UiAction::TakeScreenshot,
            Some(UiTarget::Close) => self.set_open(false),
            Some(UiTarget::Time(control)) if self.developer_controls => {
                self.time_control = control;
                UiAction::TimeChanged(control)
            }
            Some(UiTarget::Weather(control)) if self.developer_controls => {
                self.weather_control = control;
                UiAction::WeatherChanged(control)
            }
            Some(UiTarget::Spectator) if self.developer_controls && self.spectator_available => {
                UiAction::SpectatorRequested(!self.spectator_active)
            }
            Some(UiTarget::Time(_))
            | Some(UiTarget::Weather(_))
            | Some(UiTarget::Spectator)
            | Some(UiTarget::DiagnosticSky) => UiAction::None,
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
                "WORLD LAB"
            } else {
                "VOXELS / WORLD LAB"
            },
            [layout.panel.x + CONTENT_PAD, layout.header.y + 24.0],
            if layout.compact { 11.0 } else { 13.0 },
            TEXT_PRIMARY.with_alpha(opacity),
            TextAlign::Left,
        );
        push_text(
            &mut draw,
            format!(
                "{} · {} · {}",
                self.world_time_label, self.daylight_label, self.region_label
            ),
            [layout.panel.x + CONTENT_PAD, layout.header.y + 51.0],
            if layout.compact { 8.0 } else { 9.0 },
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );

        for (target, label) in [
            (UiTarget::CopyDiagnostics, "COPY INFO"),
            (UiTarget::Close, "×"),
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
                    if target == UiTarget::CopyDiagnostics {
                        8.5
                    } else {
                        17.0
                    },
                    TEXT_PRIMARY.with_alpha(opacity),
                    TextAlign::Center,
                );
            }
        }

        self.push_world_controls(&mut draw, &layout, opacity);
        self.push_movement_control(&mut draw, &layout, opacity);
        self.push_diagnostics_controls(&mut draw, &layout, opacity);
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

        draw
    }

    fn push_world_controls(&self, draw: &mut UiDrawList, layout: &UiLayout, opacity: f32) {
        push_surface(
            draw,
            layout.world_card,
            16.0,
            CARD_COLOR.with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.45),
            SurfaceRole::WorldCard,
        );
        let overridden = self.time_control != TimeControl::FollowServer
            || self.weather_control != WeatherControl::FollowServer;
        let badge = Rect::new(
            layout.world_card.x + layout.world_card.width - 108.0,
            layout.world_card.y + 10.0,
            98.0,
            20.0,
        );
        push_surface(
            draw,
            badge,
            10.0,
            if overridden { ACCENT } else { SUCCESS }.with_alpha(opacity * 0.18),
            if overridden { ACCENT } else { SUCCESS }.with_alpha(opacity * 0.72),
            SurfaceRole::AuthorityBadge,
        );
        push_text(
            draw,
            if overridden {
                "LOCAL OVERRIDE"
            } else {
                "SERVER SYNC"
            },
            badge.center(),
            7.5,
            if overridden { ACCENT } else { SUCCESS }.with_alpha(opacity),
            TextAlign::Center,
        );
        push_text(
            draw,
            "TIME OF DAY",
            [layout.world_card.x + 10.0, layout.world_card.y + 18.0],
            8.5,
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );
        self.push_segments(
            draw,
            layout,
            TimeControl::ALL.map(UiTarget::Time),
            self.time_control,
            opacity,
        );
        let first_weather = layout
            .region(UiTarget::Weather(WeatherControl::FollowServer))
            .map_or(layout.world_card.y + 100.0, |rect| rect.y);
        push_text(
            draw,
            "WEATHER",
            [layout.world_card.x + 10.0, first_weather - 12.0],
            8.5,
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );
        self.push_segments(
            draw,
            layout,
            WeatherControl::ALL.map(UiTarget::Weather),
            self.weather_control,
            opacity,
        );
    }

    fn push_segments<T: Copy + Eq>(
        &self,
        draw: &mut UiDrawList,
        layout: &UiLayout,
        targets: [UiTarget; 6],
        selected: T,
        opacity: f32,
    ) where
        UiTarget: SegmentValue<T>,
    {
        for target in targets {
            let Some(rect) = layout.region(target) else {
                continue;
            };
            let hover = self.hover_eased_value(target);
            let active = target.segment_value() == selected;
            let accent = if matches!(
                target,
                UiTarget::Time(TimeControl::FollowServer)
                    | UiTarget::Weather(WeatherControl::FollowServer)
            ) {
                SUCCESS
            } else {
                ACCENT
            };
            push_surface(
                draw,
                rect,
                rect.height * 0.5,
                if active {
                    accent.with_alpha(opacity * 0.22)
                } else {
                    CARD_COLOR
                        .mix(HOVER_COLOR, hover)
                        .with_alpha(opacity * 0.62)
                },
                if active { accent } else { PANEL_BORDER }
                    .with_alpha(opacity * (if active { 0.88 } else { 0.34 + hover * 0.5 })),
                SurfaceRole::Segment,
            );
            push_text(
                draw,
                target.segment_label(),
                rect.center(),
                if layout.compact { 8.0 } else { 8.5 },
                if active { accent } else { TEXT_PRIMARY }.with_alpha(
                    opacity
                        * if self.developer_controls || active {
                            1.0
                        } else {
                            0.45
                        },
                ),
                TextAlign::Center,
            );
        }
    }

    fn push_movement_control(&self, draw: &mut UiDrawList, layout: &UiLayout, opacity: f32) {
        let target = UiTarget::Spectator;
        let hover = self.hover_eased_value(target);
        push_surface(
            draw,
            layout.movement_card,
            16.0,
            CARD_COLOR.mix(HOVER_COLOR, hover).with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * (0.38 + hover * 0.42)),
            SurfaceRole::MovementCard,
        );
        push_text(
            draw,
            "SPECTATOR MODE",
            [layout.movement_card.x + 12.0, layout.movement_card.y + 20.0],
            10.0,
            TEXT_PRIMARY.with_alpha(opacity),
            TextAlign::Left,
        );
        let note = if !self.developer_controls {
            "Developer controls disabled in client config"
        } else if !self.spectator_available {
            "This world does not authorize spectators"
        } else if self.spectator_active {
            "WASD move · Space rise · Shift descend"
        } else {
            "Leave body here · fly without editing"
        };
        push_text(
            draw,
            note,
            [layout.movement_card.x + 12.0, layout.movement_card.y + 42.0],
            if layout.compact { 8.0 } else { 8.5 },
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );
        let track = Rect::new(
            layout.movement_card.x + layout.movement_card.width - 52.0,
            layout.movement_card.center()[1] - 11.0,
            40.0,
            22.0,
        );
        let value = self.spectator_motion.value.clamp(0.0, 1.0);
        push_surface(
            draw,
            track,
            11.0,
            TOGGLE_OFF.mix(ACCENT, value).with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.55),
            SurfaceRole::ToggleTrack,
        );
        push_surface(
            draw,
            Rect::new(track.x + 3.0 + value * 18.0, track.y + 3.0, 16.0, 16.0),
            8.0,
            TEXT_PRIMARY.with_alpha(opacity),
            Color::new(1.0, 1.0, 1.0, 0.2 * opacity),
            SurfaceRole::ToggleThumb,
        );
    }

    fn push_diagnostics_controls(&self, draw: &mut UiDrawList, layout: &UiLayout, opacity: f32) {
        push_surface(
            draw,
            layout.diagnostics_card,
            16.0,
            CARD_COLOR.with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.45),
            SurfaceRole::DiagnosticsCard,
        );
        push_text(
            draw,
            "CAPTURE / GEOMETRY",
            [
                layout.diagnostics_card.x + 12.0,
                layout.diagnostics_card.y + 18.0,
            ],
            8.5,
            TEXT_MUTED.with_alpha(opacity),
            TextAlign::Left,
        );

        if let Some(rect) = layout.region(UiTarget::DiagnosticSky) {
            let enabled_alpha = if self.developer_controls { 1.0 } else { 0.45 };
            push_text(
                draw,
                if layout.compact {
                    "DEBUG SKY"
                } else {
                    "MAGENTA DEBUG SKY"
                },
                [rect.x + 2.0, rect.y + 14.0],
                if layout.compact { 8.0 } else { 9.0 },
                TEXT_PRIMARY.with_alpha(opacity * enabled_alpha),
                TextAlign::Left,
            );
            push_text(
                draw,
                if self.developer_controls {
                    "Exposes missing geometry"
                } else {
                    "Developer controls disabled"
                },
                [rect.x + 2.0, rect.y + 33.0],
                if layout.compact { 6.5 } else { 7.5 },
                TEXT_MUTED.with_alpha(opacity * enabled_alpha),
                TextAlign::Left,
            );
            let track = Rect::new(rect.x + rect.width - 42.0, rect.y + 11.0, 40.0, 22.0);
            let value = f32::from(self.diagnostic_sky_active);
            push_surface(
                draw,
                track,
                11.0,
                TOGGLE_OFF
                    .mix(ACCENT, value)
                    .with_alpha(opacity * enabled_alpha),
                PANEL_BORDER.with_alpha(opacity * 0.55 * enabled_alpha),
                SurfaceRole::ToggleTrack,
            );
            push_surface(
                draw,
                Rect::new(track.x + 3.0 + value * 18.0, track.y + 3.0, 16.0, 16.0),
                8.0,
                TEXT_PRIMARY.with_alpha(opacity * enabled_alpha),
                Color::new(1.0, 1.0, 1.0, 0.2 * opacity * enabled_alpha),
                SurfaceRole::ToggleThumb,
            );
        }

        if let Some(rect) = layout.region(UiTarget::TakeScreenshot) {
            let hover = self.hover_eased_value(UiTarget::TakeScreenshot);
            push_surface(
                draw,
                rect,
                9.0,
                CARD_COLOR.mix(HOVER_COLOR, hover).with_alpha(opacity),
                ACCENT.with_alpha(opacity * (0.5 + hover * 0.4)),
                SurfaceRole::Button,
            );
            push_text(
                draw,
                if layout.compact {
                    "SCREENSHOT"
                } else {
                    "TAKE SCREENSHOT"
                },
                rect.center(),
                if layout.compact { 7.5 } else { 8.0 },
                TEXT_PRIMARY.with_alpha(opacity),
                TextAlign::Center,
            );
        }
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
        if !self.open && !self.spectator_active {
            self.push_inventory_wheel(draw, layout.inventory);
            self.push_edit_shape_control(draw, layout.edit_shape, layout.compact);
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
                "WASD SWIM  ·  SPACE RISE  ·  SHIFT DIVE  ·  F3 WORLD LAB"
            } else if self.stats.navigation.spectator {
                "SPECTATING  ·  WASD FLY  ·  SPACE RISE  ·  SHIFT DESCEND  ·  F3 WORLD LAB"
            } else if layout.toast.width < 500.0 {
                "WASD MOVE  ·  SPACE JUMP / GLIDE  ·  F3 WORLD LAB"
            } else {
                "CLICK TO LOOK  ·  WASD MOVE  ·  SPACE JUMP / GLIDE  ·  LMB DIG  ·  RMB PLACE  ·  F3 WORLD LAB"
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
        let launcher = if self.stats.navigation.spectator {
            format!(
                "SPECTATING  ·  {}  ·  {:.0} FPS",
                self.world_time_label, self.stats.frames_per_second
            )
        } else if self.stats.swimming {
            format!(
                "SWIMMING {:.0}%  ·  {:.1} M  ·  {:.0} FPS",
                self.stats.water_immersion * 100.0,
                self.stats.eye_depth_metres,
                self.stats.frames_per_second
            )
        } else {
            format!(
                "{}  ·  {}  ·  {:.0} FPS",
                self.world_time_label, self.daylight_label, self.stats.frames_per_second
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

    fn push_edit_shape_control(&self, draw: &mut UiDrawList, rect: Rect, compact: bool) {
        push_surface(
            draw,
            rect,
            18.0,
            PANEL_COLOR,
            PANEL_BORDER,
            SurfaceRole::Inventory,
        );
        let icon_size = if compact { 36.0 } else { 42.0 };
        let icon = Rect::new(
            rect.x
                + if compact {
                    (rect.width - icon_size) * 0.5
                } else {
                    12.0
                },
            rect.y + (rect.height - icon_size) * 0.5,
            icon_size,
            icon_size,
        );
        push_surface(
            draw,
            icon,
            if self.edit_shape == EditShape::Sphere {
                icon_size * 0.5
            } else {
                7.0
            },
            ACCENT.with_alpha(0.88),
            TEXT_PRIMARY.with_alpha(0.72),
            if self.edit_shape == EditShape::Sphere {
                SurfaceRole::InventoryOrb
            } else {
                SurfaceRole::Inventory
            },
        );
        if compact {
            push_text(
                draw,
                self.edit_shape.label(),
                [rect.center()[0], rect.y + rect.height - 7.0],
                7.0,
                TEXT_PRIMARY,
                TextAlign::Center,
            );
            return;
        }
        push_text(
            draw,
            "DIG / PLACE",
            [rect.x + 64.0, rect.y + 23.0],
            7.5,
            TEXT_MUTED,
            TextAlign::Left,
        );
        push_text(
            draw,
            self.edit_shape.label(),
            [rect.x + 64.0, rect.y + 43.0],
            11.0,
            TEXT_PRIMARY,
            TextAlign::Left,
        );
        let key = Rect::new(rect.x + rect.width - 31.0, rect.y + 9.0, 22.0, 22.0);
        push_surface(
            draw,
            key,
            6.0,
            CARD_COLOR,
            PANEL_BORDER,
            SurfaceRole::Inventory,
        );
        push_text(draw, "Q", key.center(), 9.0, ACCENT, TextAlign::Center);
    }

    fn push_inventory_wheel(&self, draw: &mut UiDrawList, rect: Rect) {
        let Some(selected) = self
            .selected_inventory_index
            .filter(|index| *index < self.inventory_items.len())
        else {
            let prompt = Rect::new(rect.center()[0] - 94.0, rect.y + 66.0, 188.0, 34.0);
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
        debug_assert!(item_count <= 10);
        let wheel_center = [rect.center()[0], rect.y + 88.0];
        let radius = [170.0, 66.0];
        for index in 0..item_count {
            let item = self.inventory_items[index];
            let offset = (index + item_count - selected) % item_count;
            let angle = -std::f32::consts::FRAC_PI_2
                + std::f32::consts::TAU * offset as f32 / item_count as f32;
            let selected_item = index == selected;
            let size = if selected_item { 46.0 } else { 30.0 };
            let orb = Rect::new(
                wheel_center[0] + angle.cos() * radius[0] - size * 0.5,
                wheel_center[1] + angle.sin() * radius[1] - size * 0.5,
                size,
                size,
            );
            push_surface(
                draw,
                orb,
                size * 0.5,
                item.color
                    .with_alpha(if selected_item { 0.96 } else { 0.72 }),
                if selected_item { ACCENT } else { PANEL_BORDER },
                SurfaceRole::InventoryOrb,
            );
            let key = Rect::new(orb.x - 4.0, orb.y - 4.0, 17.0, 17.0);
            push_surface(
                draw,
                key,
                8.5,
                PANEL_COLOR,
                if selected_item { ACCENT } else { PANEL_BORDER },
                SurfaceRole::Inventory,
            );
            push_text(
                draw,
                if index == 9 {
                    "0".to_owned()
                } else {
                    (index + 1).to_string()
                },
                key.center(),
                8.0,
                TEXT_PRIMARY,
                TextAlign::Center,
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
            [wheel_center[0], wheel_center[1] + 4.0],
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
                    "LOD TILES 0 → 7",
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
        let mut report = String::from("VOXELS / WORLD LAB\n\n");

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
        let _ = writeln!(report, "Spectator mode: {}", navigation.spectator);
        let _ = writeln!(
            report,
            "Diagnostic sky: {}",
            if self.diagnostic_sky_active {
                "magenta"
            } else {
                "off"
            }
        );

        let _ = writeln!(report, "\nWORLD");
        let _ = writeln!(
            report,
            "Time: {} ({})",
            self.world_time_label, self.daylight_label
        );
        let _ = writeln!(report, "Weather: {}", self.weather_label);
        let _ = writeln!(
            report,
            "Time authority: {}",
            if self.time_control == TimeControl::FollowServer {
                "server"
            } else {
                "local debug override"
            }
        );
        let _ = writeln!(
            report,
            "Weather authority: {}",
            if self.weather_control == WeatherControl::FollowServer {
                "server"
            } else {
                "local debug override"
            }
        );
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
            "LOD tiles 0..7: {}",
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

        report
    }

    pub fn screenshot_filename(&self) -> String {
        let navigation = self.stats.navigation;
        let [x, y, z] = navigation.eye_position_metres;
        format!(
            "voxels-x{x:.2}-y{y:.2}-z{z:.2}-h{:.1}-p{:.1}.png",
            normalized_heading(navigation.heading_degrees),
            navigation.pitch_degrees,
        )
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
        self.spectator_motion.advance(0.0, true);
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
    if stats.navigation.spectator {
        "SPECTATING"
    } else if stats.swimming {
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

    fn viewport(width: f32, height: f32) -> Viewport {
        Viewport::new(width, height, 1.0)
    }

    fn enabled(open: bool) -> MissionControlUi {
        MissionControlUi::new(MissionControlConfig {
            open,
            developer_controls: true,
            spectator_available: true,
        })
    }

    fn activate(ui: &mut MissionControlUi, target: UiTarget, viewport: Viewport) -> UiAction {
        let rect = ui.layout(viewport).region(target).expect("target region");
        ui.activate_css(rect.center(), viewport)
    }

    #[test]
    fn f3_toggles_once_per_initial_key_down() {
        let mut ui = enabled(false);
        assert_eq!(ui.handle_key(UiKey::Other, true, false), UiAction::None);
        assert_eq!(
            ui.handle_key(UiKey::F3, true, false),
            UiAction::PanelOpenChanged(true)
        );
        assert_eq!(ui.handle_key(UiKey::F3, true, true), UiAction::None);
        assert_eq!(
            ui.handle_key(UiKey::F3, true, false),
            UiAction::PanelOpenChanged(false)
        );
    }

    #[test]
    fn world_lab_layout_uses_one_source_for_paint_and_hits() {
        for viewport in [
            viewport(1_280.0, 720.0),
            viewport(640.0, 720.0),
            viewport(390.0, 844.0),
            viewport(844.0, 600.0),
        ] {
            let ui = enabled(true);
            let layout = ui.layout(viewport);
            let panel = layout.panel;
            for region in &layout.regions {
                if matches!(region.target, UiTarget::Launcher | UiTarget::EditShape) {
                    continue;
                }
                assert!(region.rect.x >= panel.x - 0.01);
                assert!(region.rect.y >= panel.y - 0.01);
                assert!(region.rect.x + region.rect.width <= panel.x + panel.width + 0.01);
                assert!(region.rect.y + region.rect.height <= panel.y + panel.height + 0.01);
                assert!(region.rect.width >= 28.0);
                assert!(region.rect.height >= 20.0);
                assert_eq!(
                    ui.hit_test_css(region.rect.center(), viewport),
                    Some(region.target)
                );
            }
            assert!(layout.world_card.y >= layout.header.y + layout.header.height);
            assert!(layout.movement_card.y >= layout.world_card.y + layout.world_card.height);
            assert!(
                layout.diagnostics_card.y >= layout.movement_card.y + layout.movement_card.height
            );
            assert!(
                layout.navigation.y >= layout.diagnostics_card.y + layout.diagnostics_card.height
            );
        }
    }

    #[test]
    fn gameplay_shape_control_cycles_by_touch_and_hides_desktop_hint_when_compact() {
        let desktop = viewport(1_280.0, 720.0);
        let mut ui = enabled(false);
        assert_eq!(ui.edit_shape(), EditShape::Sphere);
        assert_eq!(
            activate(&mut ui, UiTarget::EditShape, desktop),
            UiAction::EditShapeChanged(EditShape::Cube)
        );
        assert_eq!(ui.edit_shape(), EditShape::Cube);
        assert!(
            ui.build_draw_list(desktop)
                .text
                .iter()
                .any(|run| run.text == "Q")
        );

        let compact = viewport(390.0, 844.0);
        assert!(
            !ui.build_draw_list(compact)
                .text
                .iter()
                .any(|run| run.text == "Q")
        );
    }

    #[test]
    fn material_wheel_draws_ten_numbered_slots_with_the_selection_on_top() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = enabled(false);
        ui.set_inventory(
            Some("STONE"),
            100,
            [String::new(), String::new()],
            (0..10)
                .map(|index| InventoryItem {
                    label: "STONE",
                    count: 100 + index,
                    color: Color::new(0.2, 0.3, 0.4, 1.0),
                })
                .collect(),
            Some(3),
        );
        let layout = ui.layout(viewport);
        let draw = ui.build_draw_list(viewport);
        for label in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"] {
            assert!(draw.text.iter().any(|run| run.text == label));
        }
        let selected = draw
            .glass
            .iter()
            .find(|surface| surface.role == SurfaceRole::InventoryOrb && surface.border == ACCENT)
            .expect("selected inventory orb");
        assert!(
            selected.rect.center()[1] < layout.inventory.center()[1],
            "the selected material belongs at the top of the wheel"
        );
    }

    #[test]
    fn world_and_capture_controls_emit_typed_actions() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = enabled(true);
        assert_eq!(
            activate(&mut ui, UiTarget::Time(TimeControl::GoldenHour), viewport),
            UiAction::TimeChanged(TimeControl::GoldenHour)
        );
        assert_eq!(ui.time_control(), TimeControl::GoldenHour);
        assert_eq!(
            activate(&mut ui, UiTarget::Weather(WeatherControl::Storm), viewport),
            UiAction::WeatherChanged(WeatherControl::Storm)
        );
        assert_eq!(ui.weather_control(), WeatherControl::Storm);
        assert_eq!(
            activate(&mut ui, UiTarget::Spectator, viewport),
            UiAction::SpectatorRequested(true)
        );
        assert!(!ui.spectator_active);
        ui.set_spectator_active(true);
        assert_eq!(
            activate(&mut ui, UiTarget::Spectator, viewport),
            UiAction::SpectatorRequested(false)
        );
        assert_eq!(
            activate(&mut ui, UiTarget::Time(TimeControl::FollowServer), viewport),
            UiAction::TimeChanged(TimeControl::FollowServer)
        );
        assert_eq!(
            activate(&mut ui, UiTarget::DiagnosticSky, viewport),
            UiAction::DiagnosticSkyChanged(true)
        );
        assert!(ui.diagnostic_sky_active());
        assert_eq!(
            activate(&mut ui, UiTarget::DiagnosticSky, viewport),
            UiAction::DiagnosticSkyChanged(false)
        );
        assert_eq!(
            activate(&mut ui, UiTarget::TakeScreenshot, viewport),
            UiAction::TakeScreenshot
        );
    }

    #[test]
    fn unavailable_developer_controls_are_visible_but_inert() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = MissionControlUi::new(MissionControlConfig {
            open: true,
            developer_controls: false,
            spectator_available: false,
        });
        assert_eq!(
            activate(&mut ui, UiTarget::Time(TimeControl::Night), viewport),
            UiAction::None
        );
        assert_eq!(
            activate(&mut ui, UiTarget::Weather(WeatherControl::Rain), viewport),
            UiAction::None
        );
        assert_eq!(ui.layout(viewport).region(UiTarget::Spectator), None);
        assert_eq!(
            activate(&mut ui, UiTarget::DiagnosticSky, viewport),
            UiAction::None
        );
        assert_eq!(
            activate(&mut ui, UiTarget::TakeScreenshot, viewport),
            UiAction::TakeScreenshot
        );
        assert_eq!(ui.time_control(), TimeControl::FollowServer);
        assert_eq!(ui.weather_control(), WeatherControl::FollowServer);
    }

    #[test]
    fn draw_list_prioritizes_world_controls_and_has_no_renderer_feature_grid() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = enabled(true);
        ui.set_world_clock(0.72, "STORM", 0.9, 0.95, [5.5, 1.6], 4);
        ui.set_spectator_active(true);
        let draw = ui.build_draw_list(viewport);
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::WorldCard)
        );
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::MovementCard)
        );
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::DiagnosticsCard)
        );
        assert!(draw.text.iter().any(|run| run.text == "TAKE SCREENSHOT"));
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::Segment)
                .count(),
            TimeControl::ALL.len() + WeatherControl::ALL.len()
        );
        assert!(draw.text.iter().any(|run| run.text == "TIME OF DAY"));
        assert!(draw.text.iter().any(|run| run.text == "WEATHER"));
        assert!(draw.text.iter().any(|run| run.text.contains("Space rise")));
        assert!(!draw.text.iter().any(|run| run.text == "RENDER FEATURES"));
    }

    #[test]
    fn authority_badge_and_diagnostics_follow_override_state() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = enabled(true);
        assert!(
            ui.build_draw_list(viewport)
                .text
                .iter()
                .any(|run| run.text == "SERVER SYNC")
        );
        let _ = activate(&mut ui, UiTarget::Time(TimeControl::Night), viewport);
        let _ = activate(&mut ui, UiTarget::Weather(WeatherControl::Rain), viewport);
        assert!(
            ui.build_draw_list(viewport)
                .text
                .iter()
                .any(|run| run.text == "LOCAL OVERRIDE")
        );
        let report = ui.diagnostics_report();
        assert!(report.contains("Time authority: local debug override"));
        assert!(report.contains("Weather authority: local debug override"));
        assert!(!report.contains("RENDER FEATURES"));
    }

    #[test]
    fn world_lab_labels_every_surface_lod_level() {
        let mut ui = enabled(true);
        ui.set_stats(LiveStats {
            lod_tiles: [1, 2, 3, 4, 5, 6, 7, 8],
            ..LiveStats::default()
        });

        assert!(
            ui.card_data(false, 8)
                .iter()
                .any(|(label, value)| *label == "LOD TILES 0 → 7"
                    && value == "1 · 2 · 3 · 4 · 5 · 6 · 7 · 8")
        );
        assert!(
            ui.diagnostics_report()
                .contains("LOD tiles 0..7: 1, 2, 3, 4, 5, 6, 7, 8")
        );
    }

    #[test]
    fn player_chrome_remains_minimal_and_inventory_aware() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = enabled(false);
        ui.set_stats(LiveStats {
            frames_per_second: 120.0,
            ..LiveStats::default()
        });
        ui.set_inventory(
            Some("GRASS"),
            264,
            ["GRASS ×264".to_owned(), String::new()],
            vec![InventoryItem {
                label: "GRASS",
                count: 264,
                color: Color::new(0.2, 0.7, 0.3, 1.0),
            }],
            Some(0),
        );
        let draw = ui.build_draw_list(viewport);
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::Launcher)
        );
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::InventoryOrb)
        );
        assert!(
            draw.glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::Crosshair)
        );
        assert!(draw.text.iter().any(|run| run.text.contains("120 FPS")));
        assert!(
            !draw
                .text
                .iter()
                .any(|run| run.text.contains("MISSION CONTROL"))
        );
    }

    #[test]
    fn spectator_and_swimming_help_are_state_specific() {
        let viewport = viewport(1_280.0, 720.0);
        let mut ui = enabled(false);
        ui.set_spectator_active(true);
        ui.set_inventory(
            Some("GRASS"),
            264,
            ["GRASS ×264".to_owned(), String::new()],
            vec![InventoryItem {
                label: "GRASS",
                count: 264,
                color: Color::new(0.2, 0.7, 0.3, 1.0),
            }],
            Some(0),
        );
        ui.set_stats(LiveStats {
            navigation: NavigationTelemetry {
                spectator: true,
                ..NavigationTelemetry::default()
            },
            frames_per_second: 60.0,
            ..LiveStats::default()
        });
        let spectator_draw = ui.build_draw_list(viewport);
        assert!(
            spectator_draw
                .text
                .iter()
                .any(|run| run.text.contains("SPACE RISE") && run.text.contains("SHIFT DESCEND"))
        );
        assert!(
            !spectator_draw
                .glass
                .iter()
                .any(|surface| surface.role == SurfaceRole::InventoryOrb)
        );
        assert!(!ui.inventory_contains_css(ui.layout(viewport).inventory.center(), viewport));
        ui.set_stats(LiveStats {
            swimming: true,
            water_immersion: 0.82,
            eye_depth_metres: 0.7,
            ..LiveStats::default()
        });
        ui.show_gameplay_toast("Swimming controls");
        assert!(
            ui.build_draw_list(viewport)
                .text
                .iter()
                .any(|run| run.text.contains("Swimming controls"))
        );
    }

    #[test]
    fn screenshot_filename_identifies_the_exact_camera_pose() {
        let mut ui = enabled(true);
        ui.set_stats(LiveStats {
            navigation: NavigationTelemetry {
                eye_position_metres: [269.625, 133.625, 447.649],
                heading_degrees: -8.6,
                pitch_degrees: -15.9,
                ..NavigationTelemetry::default()
            },
            ..LiveStats::default()
        });
        assert_eq!(
            ui.screenshot_filename(),
            "voxels-x269.62-y133.62-z447.65-h351.4-p-15.9.png"
        );
    }
}
