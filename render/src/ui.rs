//! Portable state, layout, hit-testing, and draw-list generation for the in-canvas mission control.
//!
//! Coordinates in this module are CSS/device-independent pixels. The browser shell may forward raw
//! device pixels through [`Viewport::device_to_css`], but the state and resulting draw list contain
//! no browser, WASM, WGPU, or resource-lifetime concerns.

use std::collections::BTreeMap;

const PANEL_INSET: f32 = 18.0;
const PANEL_WIDTH: f32 = 360.0;
const COMPACT_WIDTH: f32 = 292.0;
const PANEL_HEIGHT: f32 = 528.0;
const COMPACT_HEIGHT: f32 = 441.0;
const HEADER_HEIGHT: f32 = 48.0;
const PANEL_RADIUS: f32 = 18.0;
const CONTENT_PAD: f32 = 14.0;
const BUTTON_SIZE: f32 = 30.0;
const BUTTON_GAP: f32 = 6.0;
const CHROME_HEIGHT: f32 = 36.0;
const BRAND_WIDTH: f32 = 178.0;
const LAUNCHER_WIDTH: f32 = 222.0;
const PANEL_TOP: f32 = PANEL_INSET + CHROME_HEIGHT + 12.0;
const CONTEXT_WIDTH: f32 = 206.0;
const CONTEXT_ROW_HEIGHT: f32 = 34.0;
const CONTEXT_PAD: f32 = 6.0;
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
    AtmosphericFog,
    FarTerrain,
    WaterSurface,
    TargetOutline,
}

impl RendererFeature {
    pub const ALL: [Self; 6] = [
        Self::CascadedSunShadows,
        Self::VoxelAmbientOcclusion,
        Self::AtmosphericFog,
        Self::FarTerrain,
        Self::WaterSurface,
        Self::TargetOutline,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::CascadedSunShadows => "Cascaded sun shadows",
            Self::VoxelAmbientOcclusion => "Voxel ambient occlusion",
            Self::AtmosphericFog => "Atmospheric fog",
            Self::FarTerrain => "Far terrain",
            Self::WaterSurface => "Animated water surface",
            Self::TargetOutline => "Target outline",
        }
    }
}

const fn feature_index(feature: RendererFeature) -> usize {
    match feature {
        RendererFeature::CascadedSunShadows => 0,
        RendererFeature::VoxelAmbientOcclusion => 1,
        RendererFeature::AtmosphericFog => 2,
        RendererFeature::FarTerrain => 3,
        RendererFeature::WaterSurface => 4,
        RendererFeature::TargetOutline => 5,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ContextAction {
    TeleportToCoast,
    DiveBelowSurface,
    ResetRendererFeatures,
    ToggleCompactTelemetry,
    HideFarTerrain,
    CloseMissionControl,
}

impl ContextAction {
    pub const ALL: [Self; 6] = [
        Self::TeleportToCoast,
        Self::DiveBelowSurface,
        Self::ResetRendererFeatures,
        Self::ToggleCompactTelemetry,
        Self::HideFarTerrain,
        Self::CloseMissionControl,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::TeleportToCoast => "Teleport to the coast",
            Self::DiveBelowSurface => "Dive below the surface",
            Self::ResetRendererFeatures => "Reset renderer features",
            Self::ToggleCompactTelemetry => "Toggle compact telemetry",
            Self::HideFarTerrain => "Hide far terrain",
            Self::CloseMissionControl => "Close mission control",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum UiTarget {
    Launcher,
    Header,
    Close,
    Compact,
    More,
    Feature(RendererFeature),
    Context(ContextAction),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiKey {
    F3,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    None,
    PanelOpenChanged(bool),
    CompactChanged(bool),
    FeatureChanged(RendererFeature, bool),
    ContextMenuOpened,
    ContextMenuClosed,
    ContextAction(ContextAction),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LiveStats {
    pub frames_per_second: f32,
    pub frame_ms: f32,
    pub cpu_ms: f32,
    pub gpu_ms: Option<f32>,
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
    pub lod_tiles: [u32; 4],
    pub pending_jobs: u32,
    pub core_gpu_bytes: u64,
    pub water_immersion: f32,
    pub eye_depth_metres: f32,
    pub eyes_submerged: bool,
    pub swimming: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceRole {
    Brand,
    Launcher,
    Toast,
    Crosshair,
    Panel,
    Header,
    Button,
    StatCard,
    FeatureRow,
    ToggleTrack,
    ToggleThumb,
    Hover,
    ContextMenu,
    ContextRow,
}

/// Renderer-agnostic description of a translucent rounded surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassSurface {
    pub rect: Rect,
    pub radius: f32,
    pub fill: Color,
    pub border: Color,
    pub blur_radius: f32,
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
    pub brand: Rect,
    pub launcher: Rect,
    pub toast: Rect,
    pub crosshair: Rect,
    pub panel: Rect,
    pub header: Rect,
    pub compact: bool,
    pub regions: Vec<InteractiveRegion>,
    pub stat_cards: Vec<Rect>,
    pub context_menu: Option<Rect>,
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
    context_anchor: Option<[f32; 2]>,
    stats: LiveStats,
    feature_enabled: [bool; 6],
    feature_motion: [EasedValue; 6],
    open_motion: EasedValue,
    compact_motion: EasedValue,
    hover_motion: BTreeMap<UiTarget, EasedValue>,
    toast_age: f32,
}

impl Default for MissionControlUi {
    fn default() -> Self {
        Self {
            open: false,
            compact: false,
            reduced_motion: false,
            hovered: None,
            context_anchor: None,
            stats: LiveStats::default(),
            feature_enabled: [true; 6],
            feature_motion: [EasedValue::new(1.0); 6],
            open_motion: EasedValue::new(0.0),
            compact_motion: EasedValue::new(0.0),
            hover_motion: BTreeMap::new(),
            toast_age: 0.0,
        }
    }
}

impl MissionControlUi {
    pub const fn open(&self) -> bool {
        self.open
    }

    pub const fn compact(&self) -> bool {
        self.compact
    }

    pub const fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub const fn hovered(&self) -> Option<UiTarget> {
        self.hovered
    }

    pub const fn context_menu_open(&self) -> bool {
        self.context_anchor.is_some()
    }

    pub const fn stats(&self) -> LiveStats {
        self.stats
    }

    pub fn set_stats(&mut self, stats: LiveStats) {
        if stats.swimming && !self.stats.swimming {
            self.toast_age = 0.0;
        }
        self.stats = stats;
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
            self.context_anchor = None;
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
        self.compact_motion.set(compact, self.reduced_motion);
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
        self.compact_motion.advance(dt, self.reduced_motion);
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
        let compact = self.effective_compact(viewport);
        let requested_width = if compact { COMPACT_WIDTH } else { PANEL_WIDTH };
        let panel_width = requested_width.min((viewport_width - PANEL_INSET * 2.0).max(180.0));
        let panel_height = if compact {
            COMPACT_HEIGHT
        } else {
            PANEL_HEIGHT
        }
        .min((viewport_height - PANEL_TOP - PANEL_INSET).max(HEADER_HEIGHT));
        let brand = Rect::new(PANEL_INSET, PANEL_INSET, BRAND_WIDTH, CHROME_HEIGHT);
        let launcher = Rect::new(
            (viewport_width - PANEL_INSET - LAUNCHER_WIDTH).max(0.0),
            PANEL_INSET,
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
        let crosshair = Rect::new(
            crosshair_center[0] - 6.0,
            crosshair_center[1] - 6.0,
            12.0,
            12.0,
        );
        let panel = Rect::new(
            (viewport_width - panel_width - PANEL_INSET).max(0.0),
            PANEL_TOP.min((viewport_height - HEADER_HEIGHT).max(0.0)),
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

        let button_y = header.y + (header.height - BUTTON_SIZE) * 0.5;
        let close = Rect::new(
            panel.x + panel.width - CONTENT_PAD - BUTTON_SIZE,
            button_y,
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        let more = Rect::new(
            close.x - BUTTON_GAP - BUTTON_SIZE,
            button_y,
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        let compact_button = Rect::new(
            more.x - BUTTON_GAP - BUTTON_SIZE,
            button_y,
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        regions.extend([
            InteractiveRegion {
                target: UiTarget::Close,
                rect: close,
            },
            InteractiveRegion {
                target: UiTarget::More,
                rect: more,
            },
            InteractiveRegion {
                target: UiTarget::Compact,
                rect: compact_button,
            },
        ]);

        let stats_top = panel.y + HEADER_HEIGHT + 14.0;
        let stat_columns: usize = if compact { 2 } else { 3 };
        let stat_gap = 7.0;
        let stat_width = (panel.width - CONTENT_PAD * 2.0 - stat_gap * (stat_columns - 1) as f32)
            / stat_columns as f32;
        let stat_height = if compact { 43.0 } else { 58.0 };
        let stat_count: usize = if compact { 6 } else { 9 };
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
        let feature_row_height = if compact { 34.0 } else { 39.0 };
        for (index, feature) in RendererFeature::ALL.into_iter().enumerate() {
            regions.push(InteractiveRegion {
                target: UiTarget::Feature(feature),
                rect: Rect::new(
                    panel.x + CONTENT_PAD,
                    feature_top + index as f32 * feature_row_height,
                    panel.width - CONTENT_PAD * 2.0,
                    feature_row_height,
                ),
            });
        }

        let context_menu = self.context_anchor.map(|anchor| {
            let height = CONTEXT_PAD * 2.0 + CONTEXT_ROW_HEIGHT * ContextAction::ALL.len() as f32;
            let x = anchor[0]
                .min(viewport_width - CONTEXT_WIDTH - PANEL_INSET)
                .max(PANEL_INSET.min(viewport_width - CONTEXT_WIDTH));
            let y = anchor[1]
                .min(viewport_height - height - PANEL_INSET)
                .max(PANEL_INSET.min(viewport_height - height));
            let menu = Rect::new(x, y, CONTEXT_WIDTH, height);
            for (index, action) in ContextAction::ALL.into_iter().enumerate() {
                regions.push(InteractiveRegion {
                    target: UiTarget::Context(action),
                    rect: Rect::new(
                        menu.x + CONTEXT_PAD,
                        menu.y + CONTEXT_PAD + index as f32 * CONTEXT_ROW_HEIGHT,
                        menu.width - CONTEXT_PAD * 2.0,
                        CONTEXT_ROW_HEIGHT,
                    ),
                });
            }
            menu
        });

        UiLayout {
            brand,
            launcher,
            toast,
            crosshair,
            panel,
            header,
            compact,
            regions,
            stat_cards,
            context_menu,
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
        if layout.context_menu.is_some() {
            return layout
                .regions
                .iter()
                .rev()
                .find(|region| {
                    matches!(region.target, UiTarget::Context(_)) && region.rect.contains(point)
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
        if self.context_anchor.is_some() {
            return match self.hit_test_css(point, viewport) {
                Some(UiTarget::Context(action)) => {
                    self.context_anchor = None;
                    self.set_hover(None);
                    UiAction::ContextAction(action)
                }
                _ => {
                    self.context_anchor = None;
                    self.set_hover(None);
                    UiAction::ContextMenuClosed
                }
            };
        }

        match self.hit_test_css(point, viewport) {
            Some(UiTarget::Launcher) => self.toggle_open(),
            Some(UiTarget::Close) => self.set_open(false),
            Some(UiTarget::Compact) => self.set_compact(!self.compact),
            Some(UiTarget::More) => {
                let layout = self.layout(viewport);
                if let Some(rect) = layout.region(UiTarget::More) {
                    self.context_anchor = Some([
                        rect.x + rect.width - CONTEXT_WIDTH,
                        rect.y + rect.height + 6.0,
                    ]);
                    UiAction::ContextMenuOpened
                } else {
                    UiAction::None
                }
            }
            Some(UiTarget::Feature(feature)) => self.toggle_feature(feature),
            Some(UiTarget::Header | UiTarget::Context(_)) | None => UiAction::None,
        }
    }

    pub fn open_context_menu_device(&mut self, point: [f32; 2], viewport: Viewport) -> UiAction {
        if !self.open {
            return UiAction::None;
        }
        self.context_anchor = Some(viewport.device_to_css(point));
        UiAction::ContextMenuOpened
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
            22.0,
            SurfaceRole::Panel,
        );
        push_surface(
            &mut draw,
            layout.header,
            PANEL_RADIUS,
            CARD_COLOR.with_alpha(opacity),
            PANEL_BORDER.with_alpha(opacity * 0.65),
            12.0,
            SurfaceRole::Header,
        );
        push_text(
            &mut draw,
            "VOXELS / MISSION CONTROL",
            [layout.panel.x + CONTENT_PAD, layout.header.center()[1]],
            if layout.compact { 11.0 } else { 12.0 },
            TEXT_PRIMARY.with_alpha(opacity),
            TextAlign::Left,
        );

        for (target, label) in [
            (UiTarget::Compact, if layout.compact { ">" } else { "<" }),
            (UiTarget::More, "..."),
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
                    8.0,
                    SurfaceRole::Button,
                );
                push_text(
                    &mut draw,
                    label,
                    rect.center(),
                    12.0,
                    TEXT_PRIMARY.with_alpha(opacity),
                    TextAlign::Center,
                );
            }
        }

        let card_data = self.card_data(layout.compact);
        for (rect, (label, value)) in layout.stat_cards.iter().copied().zip(card_data) {
            push_surface(
                &mut draw,
                rect,
                10.0,
                CARD_COLOR.with_alpha(opacity),
                PANEL_BORDER.with_alpha(opacity * 0.35),
                8.0,
                SurfaceRole::StatCard,
            );
            push_text(
                &mut draw,
                label,
                [
                    rect.x + 9.0,
                    rect.y + if layout.compact { 13.0 } else { 16.0 },
                ],
                9.0,
                TEXT_MUTED.with_alpha(opacity),
                TextAlign::Left,
            );
            push_text(
                &mut draw,
                value,
                [
                    rect.x + 9.0,
                    rect.y + if layout.compact { 30.0 } else { 40.0 },
                ],
                if layout.compact { 12.0 } else { 15.0 },
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
                6.0,
                SurfaceRole::FeatureRow,
            );
            push_text(
                &mut draw,
                feature.label(),
                [rect.x + 10.0, rect.center()[1]],
                if layout.compact { 10.5 } else { 11.5 },
                TEXT_PRIMARY.with_alpha(opacity),
                TextAlign::Left,
            );
            self.push_toggle(&mut draw, rect, feature, opacity);
        }

        if let Some(menu) = layout.context_menu {
            push_surface(
                &mut draw,
                menu,
                12.0,
                Color::new(0.025, 0.034, 0.052, 0.98).with_alpha(opacity),
                PANEL_BORDER.with_alpha(opacity),
                24.0,
                SurfaceRole::ContextMenu,
            );
            for action in ContextAction::ALL {
                let target = UiTarget::Context(action);
                let Some(rect) = layout.region(target) else {
                    continue;
                };
                let hover = self.hover_eased_value(target);
                push_surface(
                    &mut draw,
                    rect,
                    7.0,
                    Color::new(0.045, 0.060, 0.090, 0.97)
                        .mix(HOVER_COLOR, hover)
                        .with_alpha(opacity),
                    PANEL_BORDER.with_alpha(opacity * hover),
                    8.0,
                    SurfaceRole::ContextRow,
                );
                push_text(
                    &mut draw,
                    action.label(),
                    [rect.x + 10.0, rect.center()[1]],
                    10.5,
                    TEXT_PRIMARY.with_alpha(opacity),
                    TextAlign::Left,
                );
            }
        }
        draw
    }

    fn push_chrome(&self, draw: &mut UiDrawList, layout: &UiLayout) {
        push_surface(
            draw,
            layout.brand,
            CHROME_HEIGHT * 0.5,
            PANEL_COLOR,
            PANEL_BORDER,
            16.0,
            SurfaceRole::Brand,
        );
        let brand = if self.stats.eyes_submerged {
            format!("SUBMERGED / {:.1} M", self.stats.eye_depth_metres)
        } else if self.stats.water_immersion > 0.01 {
            format!("WATER / {:.0}%", self.stats.water_immersion * 100.0)
        } else {
            "VOXELS / 10 CM CUBES".into()
        };
        push_text(
            draw,
            brand,
            layout.brand.center(),
            10.5,
            TEXT_PRIMARY,
            TextAlign::Center,
        );

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
                16.0,
                SurfaceRole::Toast,
            );
            push_text(
                draw,
                if self.stats.swimming {
                    "WASD SWIM  /  SPACE ASCEND  /  SHIFT DIVE  /  F3 CONTROLS"
                } else if layout.toast.width < 500.0 {
                    "WASD MOVE  /  SPACE JUMP  /  F3 CONTROLS"
                } else {
                    "CLICK TO LOOK  /  WASD MOVE  /  SPACE JUMP  /  LMB REMOVE  /  RMB PLACE  /  F3 CONTROLS"
                },
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
            16.0,
            SurfaceRole::Launcher,
        );
        let launcher = if self.stats.swimming {
            format!(
                "SWIMMING {:.0}%   F3   {:.0} FPS",
                self.stats.water_immersion * 100.0,
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
            0.0,
            SurfaceRole::Crosshair,
        );
    }

    fn card_data(&self, compact: bool) -> Vec<(&'static str, String)> {
        let stats = self.stats;
        if compact {
            vec![
                ("FPS", format!("{:.0}", stats.frames_per_second)),
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
                ("LOAD P95", format!("{} f", stats.load_p95_frames)),
                (
                    "REMESH / EDIT",
                    format!(
                        "{} f / {:.0} ms",
                        stats.remesh_p95_frames, stats.edit_last_ms
                    ),
                ),
            ]
        } else {
            vec![
                ("FRAME", format!("{:.1} ms", stats.frame_ms)),
                (
                    "CPU / GPU",
                    format!("{:.1} / {}", stats.cpu_ms, optional_ms(stats.gpu_ms)),
                ),
                ("FPS", format!("{:.0}", stats.frames_per_second)),
                (
                    "VISIBLE / RESIDENT",
                    format!("{} / {}", stats.visible_chunks, stats.resident_chunks),
                ),
                (
                    "QUADS / WATER",
                    format!(
                        "{} / {}",
                        compact_count(u64::from(stats.quads)),
                        compact_count(u64::from(stats.water_quads)),
                    ),
                ),
                (
                    "DRAWS / WATER",
                    format!("{} / {}", stats.draw_calls, stats.water_draw_calls),
                ),
                (
                    "LOAD / REMESH / EDIT",
                    format!(
                        "{} / {} f / {:.0} ms",
                        stats.load_p95_frames, stats.remesh_p95_frames, stats.edit_last_ms
                    ),
                ),
                (
                    "JOBS + EDITS / GPU",
                    format!(
                        "{} + {} / {}",
                        stats.pending_jobs,
                        stats.edit_in_flight,
                        compact_bytes(stats.core_gpu_bytes)
                    ),
                ),
                (
                    "LOD 1 / 2 / 3 / 4",
                    format!(
                        "{} / {} / {} / {}",
                        stats.lod_tiles[0],
                        stats.lod_tiles[1],
                        stats.lod_tiles[2],
                        stats.lod_tiles[3]
                    ),
                ),
            ]
        }
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
            5.0,
            SurfaceRole::ToggleTrack,
        );
        let thumb = Rect::new(track.x + 2.0 + value * 16.0, track.y + 2.0, 14.0, 14.0);
        push_surface(
            draw,
            thumb,
            7.0,
            TEXT_PRIMARY.with_alpha(opacity),
            Color::new(1.0, 1.0, 1.0, 0.22 * opacity),
            3.0,
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
        self.compact_motion.advance(0.0, true);
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
    blur_radius: f32,
    role: SurfaceRole,
) {
    draw.glass.push(GlassSurface {
        rect,
        radius,
        fill,
        border,
        blur_radius,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Viewport {
        Viewport::new(1_280.0, 720.0, 1.0)
    }

    fn opened() -> MissionControlUi {
        let mut ui = MissionControlUi::default();
        let _ = ui.set_open(true);
        ui
    }

    #[test]
    fn f3_toggles_only_on_initial_key_down() {
        let mut ui = MissionControlUi::default();
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
        let mut ui = MissionControlUi::default();
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
        assert_eq!(normal.stat_cards.len(), 9);
        let _ = ui.set_compact(true);
        let compact = ui.layout(viewport());
        assert!(compact.compact);
        assert_eq!(compact.stat_cards.len(), 6);
        assert!(compact.panel.width < normal.panel.width);

        let automatic = opened();
        let narrow = automatic.layout(Viewport::new(640.0, 720.0, 1.0));
        assert!(narrow.compact);
        assert!(!automatic.compact());
    }

    #[test]
    fn context_menu_clamps_to_viewport_and_routes_rows() {
        let mut ui = opened();
        let viewport = viewport();
        assert_eq!(
            ui.open_context_menu_device([1_279.0, 719.0], viewport),
            UiAction::ContextMenuOpened
        );
        let layout = ui.layout(viewport);
        let menu = layout.context_menu;
        assert!(menu.is_some());
        if let Some(menu) = menu {
            assert!(menu.x + menu.width <= viewport.css_size()[0]);
            assert!(menu.y + menu.height <= viewport.css_size()[1]);
        }
        let action = ContextAction::ToggleCompactTelemetry;
        let center = layout.region(UiTarget::Context(action)).map(Rect::center);
        if let Some(center) = center {
            assert_eq!(
                ui.activate_css(center, viewport),
                UiAction::ContextAction(action)
            );
        }
        assert!(!ui.context_menu_open());
    }

    #[test]
    fn clicking_outside_context_menu_dismisses_without_action() {
        let mut ui = opened();
        let viewport = viewport();
        let _ = ui.open_context_menu_device([400.0, 300.0], viewport);
        assert_eq!(
            ui.activate_css([2.0, 2.0], viewport),
            UiAction::ContextMenuClosed
        );
        assert!(!ui.context_menu_open());
    }

    #[test]
    fn draw_list_has_reusable_glass_text_cards_toggles_and_menu_rows() {
        let mut ui = opened();
        ui.set_reduced_motion(true);
        ui.set_stats(LiveStats {
            frames_per_second: 59.8,
            frame_ms: 16.7,
            cpu_ms: 4.2,
            gpu_ms: Some(7.8),
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
            lod_tiles: [49, 49, 49, 81],
            pending_jobs: 7,
            core_gpu_bytes: 76 * 1_048_576,
            water_immersion: 0.0,
            eye_depth_metres: 0.0,
            eyes_submerged: false,
            swimming: false,
        });
        let _ = ui.open_context_menu_device([900.0, 80.0], viewport());
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
            9
        );
        assert!(draw.text.iter().any(|run| run.text == "18 / 2 f / 31 ms"));
        assert!(draw.text.iter().any(|run| run.text == "7 + 1 / 76.0 MiB"));
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::ToggleTrack)
                .count(),
            RendererFeature::ALL.len()
        );
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::ContextRow)
                .count(),
            ContextAction::ALL.len()
        );
        assert!(draw.text.iter().any(|run| run.text == "123.4k / 8.2k"));
        assert!(draw.text.iter().any(|run| run.text == "132 / 12"));
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
        assert_eq!(
            draw.glass
                .iter()
                .filter(|surface| surface.role == SurfaceRole::Brand)
                .count(),
            1
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
                .filter(|surface| surface.role == SurfaceRole::Crosshair)
                .count(),
            1
        );
    }

    #[test]
    fn launcher_is_always_hit_testable_and_opens_the_panel() {
        let mut ui = MissionControlUi::default();
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
            draw.text
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
        let mut ui = MissionControlUi::default();
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
    fn underwater_status_and_controls_are_rust_drawn() {
        let mut ui = MissionControlUi::default();
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
        assert!(draw.text.iter().any(|run| run.text == "SUBMERGED / 0.7 M"));
        assert!(
            draw.text
                .iter()
                .any(|run| run.text.contains("SPACE ASCEND") && run.text.contains("SHIFT DIVE"))
        );
        assert!(
            draw.text
                .iter()
                .any(|run| run.text.contains("SWIMMING 82%"))
        );
    }
}
