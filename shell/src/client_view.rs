#![cfg_attr(
    test,
    allow(
        dead_code,
        reason = "the native unit target tests coordinator transitions while the complete API is consumed by the WASM host"
    )
)]

use voxels_core::{CameraState, ProfileAutomation};
use voxels_render::environment::WorldEnvironmentState;
use voxels_render::renderer::{
    ClientViewPresentationState, ClientViewSession as RenderClientViewSession,
    ScreenshotMutableRenderState,
};
use voxels_runtime::{ChunkRevision, ChunkState, StreamScheduler, WorldRevisionFence};
use voxels_world::ChunkCoord;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GoalVersion(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttemptId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttemptKey {
    pub(crate) goal: GoalVersion,
    pub(crate) attempt: AttemptId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientViewSessionKind {
    Gameplay,
    Spectator,
    Profile,
    Reproduction,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum InteractiveView {
    Gameplay,
    Spectator { camera: CameraState },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProfileView {
    pub(crate) camera: CameraState,
    pub(crate) automation: ProfileAutomation,
    pub(crate) suspended: InteractiveView,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReproductionView {
    pub(crate) camera: CameraState,
    pub(crate) environment: WorldEnvironmentState,
    pub(crate) render_state: ScreenshotMutableRenderState,
    pub(crate) suspended: InteractiveView,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ClientViewSession {
    Interactive(InteractiveView),
    Profile(ProfileView),
    Reproduction(ReproductionView),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientViewState {
    gameplay_body: CameraState,
    session: ClientViewSession,
    render_state: ScreenshotMutableRenderState,
}

impl ClientViewState {
    pub(crate) const fn gameplay(
        camera: CameraState,
        render_state: ScreenshotMutableRenderState,
    ) -> Self {
        Self {
            gameplay_body: camera,
            session: ClientViewSession::Interactive(InteractiveView::Gameplay),
            render_state,
        }
    }

    pub(crate) const fn session(self) -> ClientViewSession {
        self.session
    }

    pub(crate) const fn session_kind(self) -> ClientViewSessionKind {
        match self.session {
            ClientViewSession::Interactive(InteractiveView::Gameplay) => {
                ClientViewSessionKind::Gameplay
            }
            ClientViewSession::Interactive(InteractiveView::Spectator { .. }) => {
                ClientViewSessionKind::Spectator
            }
            ClientViewSession::Profile(_) => ClientViewSessionKind::Profile,
            ClientViewSession::Reproduction(_) => ClientViewSessionKind::Reproduction,
        }
    }

    pub(crate) const fn camera(self) -> CameraState {
        match self.session {
            ClientViewSession::Interactive(InteractiveView::Gameplay) => self.gameplay_body,
            ClientViewSession::Interactive(InteractiveView::Spectator { camera }) => camera,
            ClientViewSession::Profile(profile) => profile.camera,
            ClientViewSession::Reproduction(reproduction) => reproduction.camera,
        }
    }

    pub(crate) const fn can_edit(self) -> bool {
        matches!(
            self.session,
            ClientViewSession::Interactive(InteractiveView::Gameplay)
        )
    }

    pub(crate) const fn presence_camera(self) -> Option<CameraState> {
        match self.session {
            ClientViewSession::Interactive(_) => Some(self.camera()),
            ClientViewSession::Profile(_) | ClientViewSession::Reproduction(_) => None,
        }
    }

    pub(crate) const fn presentation_state(self) -> ClientViewPresentationState {
        match self.session {
            ClientViewSession::Interactive(InteractiveView::Gameplay) => {
                ClientViewPresentationState {
                    camera: self.gameplay_body,
                    session: RenderClientViewSession::Gameplay,
                    reproduction_environment: None,
                    render_state: self.render_state,
                }
            }
            ClientViewSession::Interactive(InteractiveView::Spectator { camera }) => {
                ClientViewPresentationState {
                    camera,
                    session: RenderClientViewSession::Spectator,
                    reproduction_environment: None,
                    render_state: self.render_state,
                }
            }
            ClientViewSession::Profile(profile) => ClientViewPresentationState {
                camera: profile.camera,
                session: RenderClientViewSession::Profile,
                reproduction_environment: None,
                render_state: self.render_state,
            },
            ClientViewSession::Reproduction(reproduction) => ClientViewPresentationState {
                camera: reproduction.camera,
                session: RenderClientViewSession::Reproduction,
                reproduction_environment: Some(reproduction.environment),
                render_state: reproduction.render_state,
            },
        }
    }

    pub(crate) const fn with_render_state(
        mut self,
        render_state: ScreenshotMutableRenderState,
    ) -> Self {
        self.render_state = render_state;
        if let ClientViewSession::Reproduction(reproduction) = &mut self.session {
            reproduction.render_state = render_state;
        }
        self
    }

    pub(crate) fn with_camera(mut self, camera: CameraState) -> Self {
        match &mut self.session {
            ClientViewSession::Interactive(InteractiveView::Gameplay) => {
                self.gameplay_body = camera;
            }
            ClientViewSession::Interactive(InteractiveView::Spectator { camera: spectator }) => {
                *spectator = camera
            }
            ClientViewSession::Profile(profile) => profile.camera = camera,
            ClientViewSession::Reproduction(reproduction) => reproduction.camera = camera,
        }
        self
    }

    fn with_look_from(self, source: CameraState) -> Self {
        let mut camera = self.camera();
        camera.yaw = source.yaw;
        camera.pitch = source.pitch;
        self.with_camera(camera)
    }

    pub(crate) fn enter_spectator(mut self) -> Self {
        if matches!(
            self.session,
            ClientViewSession::Interactive(InteractiveView::Gameplay)
        ) {
            let mut camera = self.gameplay_body;
            camera.set_locomotion(voxels_core::LocomotionMode::Spectator);
            self.session = ClientViewSession::Interactive(InteractiveView::Spectator { camera });
        }
        self
    }

    pub(crate) fn leave_spectator(mut self) -> Self {
        if matches!(
            self.session,
            ClientViewSession::Interactive(InteractiveView::Spectator { .. })
        ) {
            self.session = ClientViewSession::Interactive(InteractiveView::Gameplay);
        }
        self
    }

    pub(crate) fn begin_profile(
        mut self,
        camera: CameraState,
        automation: ProfileAutomation,
    ) -> Option<Self> {
        let ClientViewSession::Interactive(suspended) = self.session else {
            return None;
        };
        self.session = ClientViewSession::Profile(ProfileView {
            camera,
            automation,
            suspended,
        });
        Some(self)
    }

    pub(crate) fn with_profile_step(
        mut self,
        camera: CameraState,
        automation: ProfileAutomation,
    ) -> Option<Self> {
        let ClientViewSession::Profile(profile) = &mut self.session else {
            return None;
        };
        profile.camera = camera;
        profile.automation = automation;
        Some(self)
    }

    pub(crate) const fn profile(self) -> Option<ProfileView> {
        match self.session {
            ClientViewSession::Profile(profile) => Some(profile),
            ClientViewSession::Interactive(_) | ClientViewSession::Reproduction(_) => None,
        }
    }

    pub(crate) fn begin_reproduction(
        mut self,
        camera: CameraState,
        environment: WorldEnvironmentState,
        render_state: ScreenshotMutableRenderState,
    ) -> Option<Self> {
        let ClientViewSession::Interactive(suspended) = self.session else {
            return None;
        };
        self.session = ClientViewSession::Reproduction(ReproductionView {
            camera,
            environment,
            render_state,
            suspended,
        });
        Some(self)
    }

    pub(crate) fn restore_interactive(mut self) -> Self {
        let suspended = match self.session {
            ClientViewSession::Interactive(_) => return self,
            ClientViewSession::Profile(profile) => profile.suspended,
            ClientViewSession::Reproduction(reproduction) => reproduction.suspended,
        };
        self.session = ClientViewSession::Interactive(suspended);
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalAttemptPlan {
    key: AttemptKey,
    collision: Box<[ChunkCoord]>,
    enclosed: Box<[ChunkCoord]>,
}

impl CanonicalAttemptPlan {
    pub(crate) fn new(
        key: AttemptKey,
        mut collision: Vec<ChunkCoord>,
        mut enclosed: Vec<ChunkCoord>,
    ) -> Self {
        canonicalize_coords(&mut collision);
        canonicalize_coords(&mut enclosed);
        Self {
            key,
            collision: collision.into_boxed_slice(),
            enclosed: enclosed.into_boxed_slice(),
        }
    }

    pub(crate) fn ready_receipt(
        &self,
        scheduler: &StreamScheduler,
    ) -> Option<CanonicalReadyReceipt> {
        let ready = self
            .collision
            .iter()
            .chain(self.enclosed.iter())
            .all(|coord| {
                scheduler.status(*coord).is_some_and(|status| {
                    status.desired && status.state == ChunkState::Resident && status.revision != 0
                })
            });
        if !ready {
            return None;
        }
        let fence = WorldRevisionFence::new(
            self.collision
                .iter()
                .chain(self.enclosed.iter())
                .filter_map(|coord| {
                    scheduler.status(*coord).map(|status| ChunkRevision {
                        coord: *coord,
                        revision: status.revision,
                    })
                }),
            std::iter::empty(),
        )
        .ok()?;
        Some(CanonicalReadyReceipt {
            key: self.key,
            fence,
        })
    }

    pub(crate) fn matches_interests(
        &self,
        collision: &[ChunkCoord],
        enclosed: &[ChunkCoord],
    ) -> bool {
        self.collision.as_ref() == collision && self.enclosed.as_ref() == enclosed
    }
}

fn canonicalize_coords(coords: &mut Vec<ChunkCoord>) {
    coords.sort_unstable();
    coords.dedup();
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalReadyReceipt {
    key: AttemptKey,
    fence: WorldRevisionFence,
}

impl CanonicalReadyReceipt {
    pub(crate) const fn key(&self) -> AttemptKey {
        self.key
    }

    pub(crate) const fn fence(&self) -> &WorldRevisionFence {
        &self.fence
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GoalSequence {
    next_goal: u64,
    next_attempt: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientViewGoalKind {
    Recenter,
    TerrainRepair,
    SpectatorEnter,
    SpectatorExit,
    ProfileStep,
    ProfileRestore,
    ReproductionApply,
    ReproductionRestore,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientViewGoal {
    version: GoalVersion,
    kind: ClientViewGoalKind,
    target: ClientViewState,
}

impl ClientViewGoal {
    pub(crate) const fn version(self) -> GoalVersion {
        self.version
    }

    pub(crate) const fn kind(self) -> ClientViewGoalKind {
        self.kind
    }

    pub(crate) const fn target(self) -> ClientViewState {
        self.target
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ClientViewAttempt<Request> {
    key: AttemptKey,
    request: Request,
    canonical: CanonicalAttemptPlan,
}

impl<Request> ClientViewAttempt<Request> {
    pub(crate) const fn key(&self) -> AttemptKey {
        self.key
    }

    pub(crate) const fn request(&self) -> &Request {
        &self.request
    }

    pub(crate) const fn canonical(&self) -> &CanonicalAttemptPlan {
        &self.canonical
    }
}

#[derive(Clone, Debug)]
struct TerminalProfileFrame<Receipt> {
    key: AttemptKey,
    published: Receipt,
}

#[derive(Clone, Debug)]
pub(crate) struct ClientViewCoordinator<Request, Receipt> {
    sequence: GoalSequence,
    current: ClientViewState,
    published: Receipt,
    goal: Option<ClientViewGoal>,
    attempt: Option<ClientViewAttempt<Request>>,
    terminal_profile_frame: Option<TerminalProfileFrame<Receipt>>,
}

impl<Request, Receipt: Copy + Eq> ClientViewCoordinator<Request, Receipt> {
    pub(crate) fn new(current: ClientViewState, published: Receipt) -> Self {
        Self {
            sequence: GoalSequence::default(),
            current,
            published,
            goal: None,
            attempt: None,
            terminal_profile_frame: None,
        }
    }

    pub(crate) const fn current(&self) -> ClientViewState {
        self.current
    }

    pub(crate) const fn camera(&self) -> CameraState {
        self.current.camera()
    }

    /// Camera whose spatial products the active attempt is preparing.
    ///
    /// The ordinary simulation and every presented-frame consumer must continue using [`Self::camera`]
    /// until this target commits. Keeping the two queries distinct prevents an unpresented proposal
    /// from leaking into physics, targeting, presence, or reproduction metadata.
    pub(crate) const fn target_camera(&self) -> CameraState {
        match self.goal {
            Some(goal) => goal.target.camera(),
            None => self.current.camera(),
        }
    }

    pub(crate) const fn published(&self) -> Receipt {
        self.published
    }

    pub(crate) fn synchronize_renderer_presentation(
        &mut self,
        state: ClientViewPresentationState,
        published: Receipt,
    ) -> bool {
        let expected = self.current.presentation_state();
        if expected.session != state.session
            || expected.reproduction_environment != state.reproduction_environment
        {
            return false;
        }
        let render_state = state.render_state;
        self.current = self
            .current
            .with_camera(state.camera)
            .with_render_state(render_state);
        if let Some(goal) = &mut self.goal {
            if goal.kind == ClientViewGoalKind::TerrainRepair {
                // A terrain-only replacement has no spatial target of its own. It tracks the
                // exact currently presented state so a slow edit rebuild cannot later restore the
                // camera pose from the frame in which its publication was first prepared.
                goal.target = self.current;
            } else if goal.kind != ClientViewGoalKind::ReproductionApply {
                // A spatial handoff owns a future position, not a stale copy of live renderer
                // controls. Keep those controls on every ordinary pending target so committing an
                // adjacent locus cannot silently undo a debug/UI/material revision.
                // ReproductionApply is deliberately excluded because its target owns the exact
                // render state embedded in the capture being reproduced.
                goal.target = goal.target.with_render_state(render_state);
            }
        }
        self.published = published;
        true
    }

    pub(crate) const fn goal(&self) -> Option<ClientViewGoal> {
        self.goal
    }

    pub(crate) const fn attempt(&self) -> Option<&ClientViewAttempt<Request>> {
        self.attempt.as_ref()
    }

    pub(crate) fn replace_goal(
        &mut self,
        kind: ClientViewGoalKind,
        target: ClientViewState,
    ) -> GoalVersion {
        let version = self.sequence.next_goal();
        self.goal = Some(ClientViewGoal {
            version,
            kind,
            target,
        });
        self.attempt = None;
        version
    }

    pub(crate) fn install_attempt(
        &mut self,
        goal: GoalVersion,
        request: Request,
        collision: Vec<ChunkCoord>,
        enclosed: Vec<ChunkCoord>,
    ) -> Option<AttemptKey> {
        if self.goal.is_none_or(|current| current.version != goal) {
            return None;
        }
        let key = self.sequence.next_attempt(goal);
        self.attempt = Some(ClientViewAttempt {
            key,
            request,
            canonical: CanonicalAttemptPlan::new(key, collision, enclosed),
        });
        Some(key)
    }

    /// Keeps the active renderer request while atomically replacing stale canonical evidence.
    ///
    /// The caller supplies already-canonical interests derived for `goal` in the current frame.
    /// Returning the extant key for an unchanged plan avoids allocation and attempt churn.
    pub(crate) fn reconcile_attempt_interests(
        &mut self,
        goal: GoalVersion,
        collision: &[ChunkCoord],
        enclosed: &[ChunkCoord],
    ) -> Option<AttemptKey>
    where
        Request: Copy,
    {
        if self.goal.is_none_or(|current| current.version != goal) {
            return None;
        }
        let attempt = self.attempt.as_ref()?;
        if attempt.canonical.matches_interests(collision, enclosed) {
            return Some(attempt.key);
        }
        let request = attempt.request;
        self.install_attempt(goal, request, collision.to_vec(), enclosed.to_vec())
    }

    pub(crate) fn commit_attempt(&mut self, key: AttemptKey, published: Receipt) -> bool {
        let Some(goal) = self.goal else {
            return false;
        };
        if goal.version != key.goal
            || self.attempt.as_ref().map(ClientViewAttempt::key) != Some(key)
        {
            return false;
        }
        self.current = goal.target;
        if goal.kind == ClientViewGoalKind::ProfileStep
            && self.current.profile().is_some_and(|profile| {
                profile.automation.phase() == voxels_core::ProfilePhase::Drain
            })
        {
            self.terminal_profile_frame = Some(TerminalProfileFrame { key, published });
        }
        self.published = published;
        self.goal = None;
        self.attempt = None;
        true
    }

    pub(crate) fn commit_in_locus(
        &mut self,
        source: Receipt,
        next: ClientViewState,
        published: Receipt,
    ) -> bool {
        if self.published != source {
            return false;
        }
        self.current = next;
        if let Some(goal) = &mut self.goal {
            if goal.kind == ClientViewGoalKind::TerrainRepair {
                goal.target = self.current;
            } else if matches!(self.current.session, ClientViewSession::Interactive(_))
                && matches!(goal.target.session, ClientViewSession::Interactive(_))
            {
                // A recenter target owns a future position, not a stale look direction. Pointer
                // input continues while that spatial handoff streams, so copy only yaw/pitch into
                // the target without leaking its unpresented position back into the current view.
                goal.target = goal.target.with_look_from(self.current.camera());
            }
        }
        self.published = published;
        true
    }

    pub(crate) fn discard_attempt(&mut self, key: AttemptKey) -> bool {
        if self.attempt.as_ref().map(ClientViewAttempt::key) != Some(key) {
            return false;
        }
        self.attempt = None;
        true
    }

    pub(crate) fn cancel_goal(&mut self, version: GoalVersion) -> bool {
        if self.goal.is_none_or(|goal| goal.version != version) {
            return false;
        }
        self.goal = None;
        self.attempt = None;
        true
    }

    pub(crate) fn terminal_profile_attempt(&self) -> Option<AttemptKey> {
        self.terminal_profile_frame
            .as_ref()
            .map(|terminal| terminal.key)
    }

    pub(crate) fn observe_terminal_submission(
        &mut self,
        submitted_matches: impl FnOnce(Receipt) -> bool,
    ) -> bool {
        let Some(terminal) = self.terminal_profile_frame.as_ref() else {
            return false;
        };
        if terminal.published != self.published || !submitted_matches(terminal.published) {
            return false;
        }
        let Some(TerminalProfileFrame {
            key: _,
            published: _,
        }) = self.terminal_profile_frame.take()
        else {
            return false;
        };
        let ClientViewSession::Profile(mut profile) = self.current.session else {
            return false;
        };
        profile.automation.complete_drain();
        self.current.session = ClientViewSession::Profile(profile);
        true
    }
}

impl Default for GoalSequence {
    fn default() -> Self {
        Self {
            next_goal: 1,
            next_attempt: 1,
        }
    }
}

impl GoalSequence {
    pub(crate) fn next_goal(&mut self) -> GoalVersion {
        let version = GoalVersion(self.next_goal);
        self.next_goal = self.next_goal.wrapping_add(1).max(1);
        version
    }

    pub(crate) fn next_attempt(&mut self, goal: GoalVersion) -> AttemptKey {
        let attempt = AttemptId(self.next_attempt);
        self.next_attempt = self.next_attempt.wrapping_add(1).max(1);
        AttemptKey { goal, attempt }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use voxels_core::LocomotionMode;

    const TEST_RENDER_STATE: ScreenshotMutableRenderState = ScreenshotMutableRenderState {
        world_lab_open: false,
        diagnostic_sky_color: None,
        geometry_source_debug: false,
        material_detail: true,
    };

    #[test]
    fn synthetic_sessions_never_replace_the_gameplay_body_or_gain_permissions() {
        let body = CameraState::spawn(Vec3::new(1.0, 2.0, 3.0));
        let state = ClientViewState::gameplay(body, TEST_RENDER_STATE);
        assert!(state.can_edit());
        assert_eq!(
            state.presence_camera().map(|camera| camera.position),
            Some(body.position)
        );

        let spectator = state.enter_spectator();
        assert_eq!(spectator.gameplay_body.position, body.position);
        assert_eq!(spectator.camera().locomotion(), LocomotionMode::Spectator);
        assert!(!spectator.can_edit());
        assert!(spectator.presence_camera().is_some());

        let profile_camera = CameraState::spawn(Vec3::new(90.0, 10.0, -20.0));
        let profile = spectator
            .begin_profile(profile_camera, ProfileAutomation::default())
            .expect("interactive spectator can be suspended");
        assert_eq!(profile.gameplay_body.position, body.position);
        assert!(!profile.can_edit());
        assert!(profile.presence_camera().is_none());

        let restored = profile.restore_interactive().leave_spectator();
        assert_eq!(restored.camera().position, body.position);
        assert!(restored.can_edit());
    }

    #[test]
    fn superseded_attempt_receipts_cannot_commit_a_previous_goal() {
        let camera_a = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let camera_b = CameraState::spawn(Vec3::new(32.0, 4.0, 0.0));
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera_a, TEST_RENDER_STATE),
            10,
        );
        let goal_b = coordinator.replace_goal(
            ClientViewGoalKind::Recenter,
            coordinator.current().with_camera(camera_b),
        );
        let attempt_b = coordinator
            .install_attempt(goal_b, 20, Vec::new(), Vec::new())
            .expect("B attempt");

        let goal_a = coordinator.replace_goal(ClientViewGoalKind::Recenter, coordinator.current());
        let attempt_a = coordinator
            .install_attempt(goal_a, 30, Vec::new(), Vec::new())
            .expect("A attempt");

        assert!(!coordinator.commit_attempt(attempt_b, 200));
        assert_eq!(coordinator.camera().position, camera_a.position);
        assert_eq!(coordinator.published(), 10);
        assert!(coordinator.commit_attempt(attempt_a, 300));
        assert_eq!(coordinator.camera().position, camera_a.position);
        assert_eq!(coordinator.published(), 300);
        assert!(!coordinator.commit_attempt(attempt_a, 301));
    }

    #[test]
    fn pending_goal_exposes_target_without_advancing_presented_camera() {
        let camera_a = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let camera_b = CameraState::spawn(Vec3::new(32.0, 4.0, -16.0));
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera_a, TEST_RENDER_STATE),
            10,
        );

        let goal = coordinator.replace_goal(
            ClientViewGoalKind::Recenter,
            coordinator.current().with_camera(camera_b),
        );
        let attempt = coordinator
            .install_attempt(goal, 20, Vec::new(), Vec::new())
            .expect("target attempt");

        assert_eq!(coordinator.camera().position, camera_a.position);
        assert_eq!(coordinator.target_camera().position, camera_b.position);
        assert!(coordinator.commit_attempt(attempt, 30));
        assert_eq!(coordinator.camera().position, camera_b.position);
        assert_eq!(coordinator.target_camera().position, camera_b.position);
    }

    #[test]
    fn in_locus_look_updates_a_pending_interactive_target_without_moving_it() {
        let camera_a = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let mut camera_b = CameraState::spawn(Vec3::new(32.0, 4.0, -16.0));
        camera_b.yaw = -0.25;
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera_a, TEST_RENDER_STATE),
            10,
        );
        coordinator.replace_goal(
            ClientViewGoalKind::Recenter,
            coordinator.current().with_camera(camera_b),
        );

        let mut looked = camera_a;
        looked.yaw = 1.25;
        looked.pitch = -0.5;
        let next = coordinator.current().with_camera(looked);
        assert!(coordinator.commit_in_locus(10, next, 11));

        assert_eq!(coordinator.camera().position, camera_a.position);
        assert_eq!(coordinator.camera().yaw, looked.yaw);
        assert_eq!(coordinator.camera().pitch, looked.pitch);
        assert_eq!(coordinator.target_camera().position, camera_b.position);
        assert_eq!(coordinator.target_camera().yaw, looked.yaw);
        assert_eq!(coordinator.target_camera().pitch, looked.pitch);
    }

    #[test]
    fn terrain_repair_tracks_in_locus_motion_instead_of_restoring_an_old_pose() {
        let camera_a = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera_a, TEST_RENDER_STATE),
            10,
        );
        let goal =
            coordinator.replace_goal(ClientViewGoalKind::TerrainRepair, coordinator.current());
        let attempt = coordinator
            .install_attempt(goal, 20, Vec::new(), Vec::new())
            .expect("terrain repair attempt");

        let mut camera_b = camera_a;
        camera_b.position = Vec3::new(2.5, 4.0, -3.0);
        camera_b.yaw = 0.7;
        camera_b.pitch = -0.4;
        assert!(coordinator.commit_in_locus(10, coordinator.current().with_camera(camera_b), 11,));
        assert_eq!(coordinator.target_camera().position, camera_b.position);
        assert_eq!(coordinator.target_camera().yaw, camera_b.yaw);
        assert_eq!(coordinator.target_camera().pitch, camera_b.pitch);

        assert!(coordinator.commit_attempt(attempt, 12));
        assert_eq!(coordinator.camera().position, camera_b.position);
        assert_eq!(coordinator.camera().yaw, camera_b.yaw);
        assert_eq!(coordinator.camera().pitch, camera_b.pitch);
    }

    #[test]
    fn moving_terrain_repair_replaces_its_canonical_plan_before_commit() {
        let camera_a = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera_a, TEST_RENDER_STATE),
            10,
        );
        let goal =
            coordinator.replace_goal(ClientViewGoalKind::TerrainRepair, coordinator.current());
        let chunk_a = ChunkCoord { x: 0, y: 0, z: 0 };
        let chunk_b = ChunkCoord { x: 0, y: 0, z: -1 };
        let attempt_a = coordinator
            .install_attempt(goal, 20, vec![chunk_a], vec![chunk_a])
            .expect("terrain repair at A");

        let mut camera_b = camera_a;
        camera_b.position.z = -3.3;
        assert!(coordinator.commit_in_locus(10, coordinator.current().with_camera(camera_b), 11,));
        assert!(
            !coordinator
                .attempt()
                .unwrap()
                .canonical()
                .matches_interests(&[chunk_b], &[chunk_b]),
            "the A certificate must not be mistaken for B evidence"
        );

        let attempt_b = coordinator
            .install_attempt(goal, 20, vec![chunk_b], vec![chunk_b])
            .expect("rebased terrain repair at B");
        assert_ne!(attempt_a, attempt_b);
        assert!(!coordinator.commit_attempt(attempt_a, 12));
        assert!(
            coordinator
                .attempt()
                .unwrap()
                .canonical()
                .matches_interests(&[chunk_b], &[chunk_b])
        );
    }

    #[test]
    fn turning_pending_recenter_replaces_its_canonical_plan_before_commit() {
        let camera_a = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let mut camera_b = CameraState::spawn(Vec3::new(32.0, 4.0, 0.0));
        camera_b.yaw = 0.25;
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera_a, TEST_RENDER_STATE),
            10,
        );
        let goal = coordinator.replace_goal(
            ClientViewGoalKind::Recenter,
            coordinator.current().with_camera(camera_b),
        );
        let forward_chunk = ChunkCoord { x: 2, y: 0, z: -1 };
        let reverse_chunk = ChunkCoord { x: 2, y: 0, z: 1 };
        let attempt_a = coordinator
            .install_attempt(goal, 20, vec![forward_chunk], vec![forward_chunk])
            .expect("recenter looking forward");

        let mut looked = camera_a;
        looked.yaw = core::f32::consts::PI;
        assert!(coordinator.commit_in_locus(10, coordinator.current().with_camera(looked), 11,));
        assert_eq!(coordinator.target_camera().position, camera_b.position);
        assert_eq!(coordinator.target_camera().yaw, looked.yaw);
        assert!(
            !coordinator
                .attempt()
                .unwrap()
                .canonical()
                .matches_interests(&[reverse_chunk], &[reverse_chunk]),
            "the forward-view certificate must not be mistaken for reverse-view evidence"
        );

        let attempt_b = coordinator
            .reconcile_attempt_interests(goal, &[reverse_chunk], &[reverse_chunk])
            .expect("production rebase after turning");
        assert_ne!(attempt_a, attempt_b);
        assert!(!coordinator.commit_attempt(attempt_a, 12));
        assert!(
            coordinator
                .attempt()
                .unwrap()
                .canonical()
                .matches_interests(&[reverse_chunk], &[reverse_chunk])
        );
    }

    #[test]
    fn renderer_owned_render_state_revision_reconciles_state_and_receipt_atomically() {
        let camera = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let current = ClientViewState::gameplay(camera, TEST_RENDER_STATE);
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(current, 10);
        let mut target_camera = camera;
        target_camera.position.z = -16.0;
        let goal = coordinator.replace_goal(
            ClientViewGoalKind::Recenter,
            current.with_camera(target_camera),
        );
        let attempt = coordinator
            .install_attempt(goal, 20, Vec::new(), Vec::new())
            .expect("pending adjacent-locus attempt");
        let revised_render_state = ScreenshotMutableRenderState {
            geometry_source_debug: true,
            ..TEST_RENDER_STATE
        };
        let renderer_state = ClientViewPresentationState {
            render_state: revised_render_state,
            ..current.presentation_state()
        };

        assert!(coordinator.synchronize_renderer_presentation(renderer_state, 11));
        assert_eq!(coordinator.published(), 11);
        assert_eq!(
            coordinator.current().presentation_state().render_state,
            revised_render_state
        );
        assert_eq!(
            coordinator
                .goal()
                .unwrap()
                .target()
                .presentation_state()
                .render_state,
            revised_render_state
        );
        assert!(coordinator.commit_attempt(attempt, 12));
        assert_eq!(
            coordinator.current().presentation_state().render_state,
            revised_render_state
        );
        assert!(coordinator.commit_in_locus(12, coordinator.current(), 13));
    }

    #[test]
    fn terminal_profile_requires_the_exact_submitted_presentation_once() {
        let camera = CameraState::spawn(Vec3::new(0.0, 4.0, 0.0));
        let mut profile = ProfileAutomation::with_config(voxels_core::ProfileConfig {
            fixed_step_seconds: 1.0,
            speed_metres_per_second: 1.0,
            warmup_seconds: 1.0,
            measure_seconds: 0.0,
        });
        profile.start(camera.position);
        profile.advance_fixed_step();
        assert_eq!(profile.phase(), voxels_core::ProfilePhase::Drain);
        let profile_state = ClientViewState::gameplay(camera, TEST_RENDER_STATE)
            .begin_profile(camera, profile)
            .expect("profile session");
        let mut coordinator = ClientViewCoordinator::<u64, u64>::new(
            ClientViewState::gameplay(camera, TEST_RENDER_STATE)
                .begin_profile(camera, ProfileAutomation::default())
                .expect("active profile session"),
            43,
        );
        let goal = coordinator.replace_goal(ClientViewGoalKind::ProfileStep, profile_state);
        let key = coordinator
            .install_attempt(goal, 7, Vec::new(), Vec::new())
            .expect("terminal profile attempt");
        assert!(coordinator.commit_attempt(key, 44));
        assert_eq!(coordinator.terminal_profile_attempt(), Some(key));
        assert!(!coordinator.observe_terminal_submission(|_| false));
        assert!(coordinator.observe_terminal_submission(|receipt| receipt == 44));
        let ClientViewSession::Profile(profile) = coordinator.current().session() else {
            panic!("profile remains the active session until its restore publishes");
        };
        assert_eq!(
            profile.automation.phase(),
            voxels_core::ProfilePhase::Complete
        );
        assert!(!coordinator.observe_terminal_submission(|_| true));
    }
}
