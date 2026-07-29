use crate::CameraState;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct IntentId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PresentationGeneration(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SceneToken(pub u64);

/// Identity for one committed terrain/scene presentation target.
///
/// Bounded camera motion may retain this token while it remains inside that presentation contract.
/// A transition destination or changed terrain/scene target must use a freshly planned token. The
/// reducer never compares floating-point cameras to decide whether asynchronous work is current.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PresentationToken(u64);

impl PresentationToken {
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ContractIdentity(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ContractRevision(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationContract {
    pub identity: ContractIdentity,
    pub revision: ContractRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewOwner {
    Gameplay,
    Spectator,
    Profile,
    Reproduction,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewDestination {
    pub owner: ViewOwner,
    pub camera: CameraState,
    pub scene: SceneToken,
}

impl ViewDestination {
    pub const fn new(owner: ViewOwner, camera: CameraState, scene: SceneToken) -> Self {
        Self {
            owner,
            camera,
            scene,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewDescriptor {
    pub owner: ViewOwner,
    pub camera: CameraState,
    pub scene: SceneToken,
    pub presentation: PresentationToken,
}

impl ViewDescriptor {
    const fn from_destination(
        destination: ViewDestination,
        presentation: PresentationToken,
    ) -> Self {
        Self {
            owner: destination.owner,
            camera: destination.camera,
            scene: destination.scene,
            presentation,
        }
    }

    const fn destination(self) -> ViewDestination {
        ViewDestination {
            owner: self.owner,
            camera: self.camera,
            scene: self.scene,
        }
    }
}

/// The durable multiplayer body. It is never moved into an intent, staging token, or view session.
#[derive(Clone, Copy, Debug)]
pub struct AuthoritativeBody {
    camera: CameraState,
    scene: SceneToken,
    contract_identity: ContractIdentity,
}

impl AuthoritativeBody {
    pub const fn new(
        camera: CameraState,
        scene: SceneToken,
        contract_identity: ContractIdentity,
    ) -> Self {
        Self {
            camera,
            scene,
            contract_identity,
        }
    }

    pub const fn camera(self) -> CameraState {
        self.camera
    }

    pub const fn scene(self) -> SceneToken {
        self.scene
    }

    pub const fn contract_identity(self) -> ContractIdentity {
        self.contract_identity
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivePresentation {
    pub view: ViewDescriptor,
    pub generation: PresentationGeneration,
    pub contract: PresentationContract,
}

/// One-use proof that a candidate camera was checked against the exact active presentation.
///
/// Fields and construction are private, so another crate cannot forge or retarget an admission.
#[derive(Debug)]
pub struct MovementAdmission {
    candidate: CameraState,
    owner: ViewOwner,
    generation: PresentationGeneration,
    scene: SceneToken,
    presentation: PresentationToken,
    contract: PresentationContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorityLease {
    Gameplay,
    SpectatorView,
    Denied,
}

impl AuthorityLease {
    pub const fn allows_body_simulation(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    pub const fn allows_edits(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    pub const fn allows_spectator_view_input(self) -> bool {
        matches!(self, Self::SpectatorView)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileSessionPhase {
    Running,
    Drain {
        terminal_generation: PresentationGeneration,
        frame_submitted: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct ProfileSession {
    committed_ticks: u64,
    phase: ProfileSessionPhase,
}

impl ProfileSession {
    pub const fn committed_ticks(self) -> u64 {
        self.committed_ticks
    }

    pub const fn phase(self) -> ProfileSessionPhase {
        self.phase
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ViewSession {
    Gameplay,
    Spectator,
    Profile(ProfileSession),
    Reproduction,
}

impl ViewSession {
    pub const fn owner(self) -> ViewOwner {
        match self {
            Self::Gameplay => ViewOwner::Gameplay,
            Self::Spectator => ViewOwner::Spectator,
            Self::Profile(_) => ViewOwner::Profile,
            Self::Reproduction => ViewOwner::Reproduction,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntentEffect {
    EnterSpectator,
    ExitSpectator,
    ForcedSpectatorReturn,
    StartProfile,
    ProfileTick { tick: u64, terminal: bool },
    ReturnFromProfile,
    ApplyReproduction,
    ReplaceReproduction,
    ClearReproduction,
}

#[derive(Clone, Copy, Debug)]
pub struct DesiredIntent {
    pub id: IntentId,
    pub source_generation: PresentationGeneration,
    pub destination: ViewDescriptor,
    pub contract: PresentationContract,
    effect: IntentEffect,
}

impl DesiredIntent {
    pub const fn staging_token(self) -> StagingToken {
        StagingToken {
            intent: self.id,
            source_generation: self.source_generation,
            destination_owner: self.destination.owner,
            destination_scene: self.destination.scene,
            destination_presentation: self.destination.presentation,
            contract: self.contract,
        }
    }

    fn matches(
        self,
        destination: ViewDestination,
        contract: PresentationContract,
        effect: IntentEffect,
    ) -> bool {
        self.destination.destination() == destination
            && self.contract == contract
            && self.effect == effect
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StagingToken {
    pub intent: IntentId,
    pub source_generation: PresentationGeneration,
    pub destination_owner: ViewOwner,
    pub destination_scene: SceneToken,
    pub destination_presentation: PresentationToken,
    pub contract: PresentationContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedCompletion {
    token: StagingToken,
}

impl PreparedCompletion {
    pub const fn token(self) -> StagingToken {
        self.token
    }
}

#[derive(Debug)]
pub enum ViewEvent {
    UpdateGameplayBody(MovementAdmission),
    UpdateSpectatorView(MovementAdmission),
    /// Explicit request after the host has confirmed spectator capability. This is also the only
    /// event which can cancel a capability-forced return.
    EnterSpectator {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    ExitSpectator {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    SpectatorCapabilityLost {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    StartProfile {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    ProfileTick {
        tick: u64,
        destination: ViewDestination,
        terminal: bool,
        contract: PresentationContract,
    },
    ProfileTerminalFrameSubmitted {
        generation: PresentationGeneration,
        destination: ViewDestination,
        contract: PresentationContract,
    },
    ProfileCancel {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    ApplyReproduction {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    ClearReproduction {
        destination: ViewDestination,
        contract: PresentationContract,
    },
    ContractInvalidated(PresentationContract),
    Commit(PreparedCompletion),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReducerEffect {
    Unchanged,
    BodyUpdated,
    ViewUpdated,
    IntentStarted(IntentId),
    IntentCancelled(IntentId),
    PresentationCommitted(PresentationGeneration),
    AuthorityRevoked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewError {
    AuthorityDenied,
    ConflictingSession,
    TransitionInFlight,
    NoTransitionInFlight,
    StaleCompletion,
    WrongDestinationOwner {
        expected: ViewOwner,
        actual: ViewOwner,
    },
    WrongProfileTick {
        expected: u64,
        actual: u64,
    },
    ProfileNotDraining,
    WrongTerminalGeneration {
        expected: PresentationGeneration,
        actual: PresentationGeneration,
    },
    StaleMovementAdmission,
    MovementOutsideContract,
    WrongGameplayScene {
        expected: SceneToken,
        actual: SceneToken,
    },
    WrongGameplayContractIdentity {
        expected: ContractIdentity,
        actual: ContractIdentity,
    },
    InvalidPresentationContract,
    SequenceExhausted,
}

/// Pure owner/intent reducer for every exceptional camera transition.
///
/// The active presentation is always the last committed owner/camera/token tuple. A desired
/// destination never mutates that tuple, and the durable authoritative body remains separate even
/// while another owner controls the visible camera.
#[derive(Clone, Debug)]
pub struct ViewController {
    body: AuthoritativeBody,
    active: ActivePresentation,
    session: SessionState,
    desired: Option<DesiredIntent>,
    next_intent: u64,
    next_presentation: u64,
    active_contract_valid: bool,
    poisoned_intent: Option<IntentId>,
}

#[derive(Clone, Copy, Debug)]
enum SessionState {
    Gameplay,
    Spectator,
    Profile(ProfileSession),
    Reproduction,
}

impl SessionState {
    const fn public(self) -> ViewSession {
        match self {
            Self::Gameplay => ViewSession::Gameplay,
            Self::Spectator => ViewSession::Spectator,
            Self::Profile(profile) => ViewSession::Profile(profile),
            Self::Reproduction => ViewSession::Reproduction,
        }
    }

    const fn owner(self) -> ViewOwner {
        self.public().owner()
    }
}

impl ViewController {
    pub fn new(body: CameraState, scene: SceneToken, contract: PresentationContract) -> Self {
        Self {
            body: AuthoritativeBody::new(body, scene, contract.identity),
            active: ActivePresentation {
                view: ViewDescriptor::from_destination(
                    ViewDestination::new(ViewOwner::Gameplay, body, scene),
                    PresentationToken(1),
                ),
                generation: PresentationGeneration(1),
                contract,
            },
            session: SessionState::Gameplay,
            desired: None,
            next_intent: 1,
            next_presentation: 2,
            active_contract_valid: true,
            poisoned_intent: None,
        }
    }

    pub const fn authoritative_body(&self) -> AuthoritativeBody {
        self.body
    }

    pub const fn active_presentation(&self) -> ActivePresentation {
        self.active
    }

    pub const fn session(&self) -> ViewSession {
        self.session.public()
    }

    pub const fn desired_intent(&self) -> Option<DesiredIntent> {
        self.desired
    }

    pub fn authority_lease(&self) -> AuthorityLease {
        if self.desired.is_some() || !self.active_contract_valid {
            return AuthorityLease::Denied;
        }
        match self.session {
            SessionState::Gameplay => AuthorityLease::Gameplay,
            SessionState::Spectator => AuthorityLease::SpectatorView,
            SessionState::Profile(_) | SessionState::Reproduction => AuthorityLease::Denied,
        }
    }

    /// Shutdown never persists an active or desired view camera.
    pub const fn shutdown_camera(&self) -> CameraState {
        self.body.camera()
    }

    /// Produces a one-use camera admission only after the host verifies the candidate against the
    /// complete active owner/camera/scene/presentation/generation/contract tuple.
    pub fn admit_movement(
        &self,
        candidate: CameraState,
        verifier: impl FnOnce(ActivePresentation, CameraState) -> bool,
    ) -> Result<MovementAdmission, ViewError> {
        if matches!(self.authority_lease(), AuthorityLease::Denied) {
            return Err(ViewError::AuthorityDenied);
        }
        if !verifier(self.active, candidate) {
            return Err(ViewError::MovementOutsideContract);
        }
        Ok(MovementAdmission {
            candidate,
            owner: self.active.view.owner,
            generation: self.active.generation,
            scene: self.active.view.scene,
            presentation: self.active.view.presentation,
            contract: self.active.contract,
        })
    }

    pub fn prepare_completion(&self, token: StagingToken) -> Result<PreparedCompletion, ViewError> {
        let desired = self.desired.ok_or(ViewError::NoTransitionInFlight)?;
        if desired.staging_token() != token {
            return Err(ViewError::StaleCompletion);
        }
        if self.poisoned_intent == Some(desired.id) {
            return Err(ViewError::StaleCompletion);
        }
        Ok(PreparedCompletion { token })
    }

    pub fn reduce(&mut self, event: ViewEvent) -> Result<ReducerEffect, ViewError> {
        match event {
            ViewEvent::UpdateGameplayBody(admission) => self.update_gameplay_body(admission),
            ViewEvent::UpdateSpectatorView(admission) => self.update_spectator_view(admission),
            ViewEvent::EnterSpectator {
                destination,
                contract,
            } => self.enter_spectator(destination, contract),
            ViewEvent::ExitSpectator {
                destination,
                contract,
            } => self.exit_spectator(destination, contract),
            ViewEvent::SpectatorCapabilityLost {
                destination,
                contract,
            } => self.spectator_capability_lost(destination, contract),
            ViewEvent::ProfileCancel {
                destination,
                contract,
            } => self.cancel_profile(destination, contract),
            ViewEvent::ProfileTerminalFrameSubmitted {
                generation,
                destination,
                contract,
            } => self.profile_terminal_frame_submitted(generation, destination, contract),
            ViewEvent::StartProfile {
                destination,
                contract,
            } => self.start_profile(destination, contract),
            ViewEvent::ProfileTick {
                tick,
                destination,
                terminal,
                contract,
            } => self.profile_tick(tick, destination, terminal, contract),
            ViewEvent::ApplyReproduction {
                destination,
                contract,
            } => self.apply_reproduction(destination, contract),
            ViewEvent::ClearReproduction {
                destination,
                contract,
            } => self.clear_reproduction(destination, contract),
            ViewEvent::ContractInvalidated(contract) => self.invalidate_contract(contract),
            ViewEvent::Commit(completion) => self.commit(completion),
        }
    }

    fn update_gameplay_body(
        &mut self,
        admission: MovementAdmission,
    ) -> Result<ReducerEffect, ViewError> {
        if !self.authority_lease().allows_body_simulation() {
            return Err(ViewError::AuthorityDenied);
        }
        let camera = self.validate_movement_admission(admission)?;
        self.body.camera = camera;
        self.active.view.camera = camera;
        Ok(ReducerEffect::BodyUpdated)
    }

    fn update_spectator_view(
        &mut self,
        admission: MovementAdmission,
    ) -> Result<ReducerEffect, ViewError> {
        if !self.authority_lease().allows_spectator_view_input() {
            return Err(ViewError::AuthorityDenied);
        }
        let camera = self.validate_movement_admission(admission)?;
        self.active.view.camera = camera;
        Ok(ReducerEffect::ViewUpdated)
    }

    fn enter_spectator(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        Self::expect_owner(destination, ViewOwner::Spectator)?;
        match (self.session, self.desired) {
            (SessionState::Gameplay, None) => {
                self.start_intent(destination, contract, IntentEffect::EnterSpectator)
            }
            (SessionState::Spectator, Some(desired))
                if matches!(
                    desired.effect,
                    IntentEffect::ExitSpectator | IntentEffect::ForcedSpectatorReturn
                ) =>
            {
                self.cancel_intent()
            }
            (SessionState::Spectator, None) => Ok(ReducerEffect::Unchanged),
            (_, Some(_)) => Err(ViewError::TransitionInFlight),
            _ => Err(ViewError::ConflictingSession),
        }
    }

    fn exit_spectator(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        let destination = self.gameplay_destination(destination, contract)?;
        match (self.session, self.desired) {
            (SessionState::Gameplay, Some(desired))
                if matches!(desired.effect, IntentEffect::EnterSpectator) =>
            {
                self.cancel_intent()
            }
            (SessionState::Spectator, None) => {
                self.start_intent(destination, contract, IntentEffect::ExitSpectator)
            }
            (_, Some(_)) => Err(ViewError::TransitionInFlight),
            _ => Err(ViewError::ConflictingSession),
        }
    }

    fn spectator_capability_lost(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        let destination = self.gameplay_destination(destination, contract)?;
        match (self.session, self.desired) {
            (SessionState::Gameplay, Some(desired))
                if matches!(desired.effect, IntentEffect::EnterSpectator) =>
            {
                self.cancel_intent()
            }
            (SessionState::Spectator, None) => {
                self.start_intent(destination, contract, IntentEffect::ForcedSpectatorReturn)
            }
            (SessionState::Spectator, Some(desired))
                if matches!(
                    desired.effect,
                    IntentEffect::ExitSpectator | IntentEffect::ForcedSpectatorReturn
                ) =>
            {
                self.start_intent(destination, contract, IntentEffect::ForcedSpectatorReturn)
            }
            (_, Some(_)) => Err(ViewError::TransitionInFlight),
            _ => Ok(ReducerEffect::Unchanged),
        }
    }

    fn start_profile(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        Self::expect_owner(destination, ViewOwner::Profile)?;
        if self.desired.is_some() {
            return Err(ViewError::TransitionInFlight);
        }
        if !matches!(self.session, SessionState::Gameplay) {
            return Err(ViewError::ConflictingSession);
        }
        self.start_intent(destination, contract, IntentEffect::StartProfile)
    }

    fn profile_tick(
        &mut self,
        tick: u64,
        destination: ViewDestination,
        terminal: bool,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        Self::expect_owner(destination, ViewOwner::Profile)?;
        if self.desired.is_some() {
            return Err(ViewError::TransitionInFlight);
        }
        let SessionState::Profile(profile) = self.session else {
            return Err(ViewError::ConflictingSession);
        };
        if profile.phase != ProfileSessionPhase::Running {
            return Err(ViewError::ProfileNotDraining);
        }
        let expected = profile
            .committed_ticks
            .checked_add(1)
            .ok_or(ViewError::SequenceExhausted)?;
        if tick != expected {
            return Err(ViewError::WrongProfileTick {
                expected,
                actual: tick,
            });
        }
        self.start_intent(
            destination,
            contract,
            IntentEffect::ProfileTick { tick, terminal },
        )
    }

    fn profile_terminal_frame_submitted(
        &mut self,
        generation: PresentationGeneration,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        let destination = self.gameplay_destination(destination, contract)?;
        if self.desired.is_some() {
            return Err(ViewError::TransitionInFlight);
        }
        let SessionState::Profile(mut profile) = self.session else {
            return Err(ViewError::ConflictingSession);
        };
        let ProfileSessionPhase::Drain {
            terminal_generation,
            frame_submitted: false,
        } = profile.phase
        else {
            return Err(ViewError::ProfileNotDraining);
        };
        if generation != terminal_generation || generation != self.active.generation {
            return Err(ViewError::WrongTerminalGeneration {
                expected: terminal_generation,
                actual: generation,
            });
        }
        profile.phase = ProfileSessionPhase::Drain {
            terminal_generation,
            frame_submitted: true,
        };
        let effect = self.start_intent(destination, contract, IntentEffect::ReturnFromProfile)?;
        self.session = SessionState::Profile(profile);
        Ok(effect)
    }

    fn cancel_profile(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        let destination = self.gameplay_destination(destination, contract)?;
        match (self.session, self.desired) {
            (SessionState::Gameplay, Some(desired))
                if matches!(desired.effect, IntentEffect::StartProfile) =>
            {
                self.cancel_intent()
            }
            (SessionState::Profile(_), None) => {
                self.start_intent(destination, contract, IntentEffect::ReturnFromProfile)
            }
            (SessionState::Profile(_), Some(desired))
                if matches!(
                    desired.effect,
                    IntentEffect::ProfileTick { .. } | IntentEffect::ReturnFromProfile
                ) =>
            {
                self.start_intent(destination, contract, IntentEffect::ReturnFromProfile)
            }
            (_, Some(_)) => Err(ViewError::TransitionInFlight),
            _ => Err(ViewError::ConflictingSession),
        }
    }

    fn apply_reproduction(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        Self::expect_owner(destination, ViewOwner::Reproduction)?;
        match self.session {
            SessionState::Gameplay => match self.desired {
                None => self.start_intent(destination, contract, IntentEffect::ApplyReproduction),
                Some(desired) if matches!(desired.effect, IntentEffect::ApplyReproduction) => {
                    self.start_intent(destination, contract, IntentEffect::ApplyReproduction)
                }
                Some(_) => Err(ViewError::TransitionInFlight),
            },
            SessionState::Reproduction => {
                if self.desired.is_some_and(|desired| {
                    !matches!(
                        desired.effect,
                        IntentEffect::ReplaceReproduction | IntentEffect::ClearReproduction
                    )
                }) {
                    return Err(ViewError::TransitionInFlight);
                }
                self.start_intent(destination, contract, IntentEffect::ReplaceReproduction)
            }
            _ if self.desired.is_some() => Err(ViewError::TransitionInFlight),
            _ => Err(ViewError::ConflictingSession),
        }
    }

    fn clear_reproduction(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        let destination = self.gameplay_destination(destination, contract)?;
        match self.session {
            SessionState::Gameplay => {
                if self.desired.is_some_and(|desired| {
                    matches!(desired.effect, IntentEffect::ApplyReproduction)
                }) {
                    self.cancel_intent()
                } else if self.desired.is_some() {
                    Err(ViewError::TransitionInFlight)
                } else {
                    Err(ViewError::ConflictingSession)
                }
            }
            SessionState::Reproduction => {
                if self.desired.is_some_and(|desired| {
                    !matches!(
                        desired.effect,
                        IntentEffect::ReplaceReproduction | IntentEffect::ClearReproduction
                    )
                }) {
                    return Err(ViewError::TransitionInFlight);
                }
                self.start_intent(destination, contract, IntentEffect::ClearReproduction)
            }
            _ if self.desired.is_some() => Err(ViewError::TransitionInFlight),
            _ => Err(ViewError::ConflictingSession),
        }
    }

    fn invalidate_contract(
        &mut self,
        contract: PresentationContract,
    ) -> Result<ReducerEffect, ViewError> {
        let mut revoked = false;
        if self.active.contract == contract && self.active_contract_valid {
            self.active_contract_valid = false;
            revoked = true;
        }
        if let Some(desired) = self.desired.filter(|desired| desired.contract == contract) {
            self.poisoned_intent = Some(desired.id);
        }
        if revoked {
            Ok(ReducerEffect::AuthorityRevoked)
        } else {
            Ok(ReducerEffect::Unchanged)
        }
    }

    fn commit(&mut self, completion: PreparedCompletion) -> Result<ReducerEffect, ViewError> {
        let desired = self.desired.ok_or(ViewError::NoTransitionInFlight)?;
        if desired.staging_token() != completion.token {
            return Err(ViewError::StaleCompletion);
        }
        if self.poisoned_intent == Some(desired.id) {
            return Err(ViewError::StaleCompletion);
        }
        let generation = PresentationGeneration(
            self.active
                .generation
                .0
                .checked_add(1)
                .ok_or(ViewError::SequenceExhausted)?,
        );
        self.session = match desired.effect {
            IntentEffect::EnterSpectator => SessionState::Spectator,
            IntentEffect::ExitSpectator | IntentEffect::ForcedSpectatorReturn => {
                SessionState::Gameplay
            }
            IntentEffect::StartProfile => SessionState::Profile(ProfileSession {
                committed_ticks: 0,
                phase: ProfileSessionPhase::Running,
            }),
            IntentEffect::ProfileTick { tick, terminal } => {
                let SessionState::Profile(mut profile) = self.session else {
                    return Err(ViewError::StaleCompletion);
                };
                let expected = profile
                    .committed_ticks
                    .checked_add(1)
                    .ok_or(ViewError::SequenceExhausted)?;
                if tick != expected || profile.phase != ProfileSessionPhase::Running {
                    return Err(ViewError::StaleCompletion);
                }
                profile.committed_ticks = tick;
                if terminal {
                    profile.phase = ProfileSessionPhase::Drain {
                        terminal_generation: generation,
                        frame_submitted: false,
                    };
                }
                SessionState::Profile(profile)
            }
            IntentEffect::ReturnFromProfile => SessionState::Gameplay,
            IntentEffect::ApplyReproduction => SessionState::Reproduction,
            IntentEffect::ReplaceReproduction => self.session,
            IntentEffect::ClearReproduction => SessionState::Gameplay,
        };
        self.active = ActivePresentation {
            view: desired.destination,
            generation,
            contract: desired.contract,
        };
        self.active_contract_valid = true;
        self.poisoned_intent = None;
        self.desired = None;
        debug_assert_eq!(self.active.view.owner, self.session.owner());
        if matches!(self.session, SessionState::Gameplay) {
            debug_assert_eq!(self.active.view.owner, ViewOwner::Gameplay);
        }
        Ok(ReducerEffect::PresentationCommitted(generation))
    }

    fn start_intent(
        &mut self,
        destination: ViewDestination,
        contract: PresentationContract,
        effect: IntentEffect,
    ) -> Result<ReducerEffect, ViewError> {
        if !self.active_contract_valid && self.active.contract == contract {
            return Err(ViewError::InvalidPresentationContract);
        }
        if self
            .desired
            .is_some_and(|desired| desired.matches(destination, contract, effect))
        {
            return Ok(ReducerEffect::Unchanged);
        }
        let id = IntentId(self.next_intent);
        let next_intent = self
            .next_intent
            .checked_add(1)
            .ok_or(ViewError::SequenceExhausted)?;
        let presentation = PresentationToken(self.next_presentation);
        let next_presentation = self
            .next_presentation
            .checked_add(1)
            .ok_or(ViewError::SequenceExhausted)?;
        self.next_intent = next_intent;
        self.next_presentation = next_presentation;
        self.poisoned_intent = None;
        self.desired = Some(DesiredIntent {
            id,
            source_generation: self.active.generation,
            destination: ViewDescriptor::from_destination(destination, presentation),
            contract,
            effect,
        });
        Ok(ReducerEffect::IntentStarted(id))
    }

    fn cancel_intent(&mut self) -> Result<ReducerEffect, ViewError> {
        let intent = self.desired.take().ok_or(ViewError::NoTransitionInFlight)?;
        self.poisoned_intent = None;
        Ok(ReducerEffect::IntentCancelled(intent.id))
    }

    fn gameplay_destination(
        &self,
        mut destination: ViewDestination,
        contract: PresentationContract,
    ) -> Result<ViewDestination, ViewError> {
        Self::expect_owner(destination, ViewOwner::Gameplay)?;
        if destination.scene != self.body.scene {
            return Err(ViewError::WrongGameplayScene {
                expected: self.body.scene,
                actual: destination.scene,
            });
        }
        if contract.identity != self.body.contract_identity {
            return Err(ViewError::WrongGameplayContractIdentity {
                expected: self.body.contract_identity,
                actual: contract.identity,
            });
        }
        destination.camera = self.body.camera();
        Ok(destination)
    }

    fn validate_movement_admission(
        &self,
        admission: MovementAdmission,
    ) -> Result<CameraState, ViewError> {
        if admission.owner != self.active.view.owner
            || admission.generation != self.active.generation
            || admission.scene != self.active.view.scene
            || admission.presentation != self.active.view.presentation
            || admission.contract != self.active.contract
        {
            return Err(ViewError::StaleMovementAdmission);
        }
        Ok(admission.candidate)
    }

    fn expect_owner(destination: ViewDestination, expected: ViewOwner) -> Result<(), ViewError> {
        if destination.owner == expected {
            Ok(())
        } else {
            Err(ViewError::WrongDestinationOwner {
                expected,
                actual: destination.owner,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn camera(marker: f32) -> CameraState {
        let mut camera = CameraState::spawn(Vec3::new(marker, marker + 8.0, -marker));
        camera.yaw = marker * 0.01;
        camera
    }

    fn view(owner: ViewOwner, marker: u64) -> ViewDestination {
        ViewDestination::new(
            owner,
            camera(marker as f32),
            SceneToken(marker.saturating_mul(10).saturating_add(1)),
        )
    }

    fn gameplay_view(marker: u64) -> ViewDestination {
        ViewDestination::new(ViewOwner::Gameplay, camera(marker as f32), SceneToken(11))
    }

    const fn contract(identity: u64, revision: u64) -> PresentationContract {
        PresentationContract {
            identity: ContractIdentity(identity),
            revision: ContractRevision(revision),
        }
    }

    fn controller() -> ViewController {
        ViewController::new(camera(1.0), SceneToken(11), contract(100, 1))
    }

    fn admit(controller: &ViewController, candidate: CameraState) -> MovementAdmission {
        controller
            .admit_movement(candidate, |_, _| true)
            .expect("test candidate is admitted")
    }

    fn prepare_current(controller: &ViewController) -> PreparedCompletion {
        let token = controller
            .desired_intent()
            .expect("test transition must have a desired intent")
            .staging_token();
        controller
            .prepare_completion(token)
            .expect("current staging token must prepare")
    }

    fn commit_current(controller: &mut ViewController) -> PresentationGeneration {
        let prepared = prepare_current(controller);
        let ReducerEffect::PresentationCommitted(generation) = controller
            .reduce(ViewEvent::Commit(prepared))
            .expect("current prepared completion must commit")
        else {
            panic!("commit returned the wrong reducer effect");
        };
        generation
    }

    fn position(controller: &ViewController) -> Vec3 {
        controller.authoritative_body().camera().position
    }

    fn assert_invariants(controller: &ViewController) {
        let active = controller.active_presentation();
        assert_eq!(active.view.owner, controller.session().owner());
        assert_eq!(controller.shutdown_camera().position, position(controller));
        if let Some(desired) = controller.desired_intent() {
            assert_eq!(desired.source_generation, active.generation);
            assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        } else if !controller.active_contract_valid {
            assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        } else {
            let expected = match controller.session() {
                ViewSession::Gameplay => AuthorityLease::Gameplay,
                ViewSession::Spectator => AuthorityLease::SpectatorView,
                ViewSession::Profile(_) | ViewSession::Reproduction => AuthorityLease::Denied,
            };
            assert_eq!(controller.authority_lease(), expected);
        }
        if matches!(controller.session(), ViewSession::Gameplay) {
            assert_eq!(active.view.camera.position, position(controller));
        }
    }

    #[test]
    fn desired_camera_b_never_mutates_active_camera_a_before_commit() {
        let mut controller = controller();
        let active_a = controller.active_presentation();
        let body_a = position(&controller);
        let destination_b = view(ViewOwner::Spectator, 2);

        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: destination_b,
                contract: contract(1, 1),
            })
            .unwrap();

        assert_eq!(
            controller.active_presentation().view.camera.position,
            active_a.view.camera.position
        );
        assert_eq!(position(&controller), body_a);
        assert_eq!(
            controller
                .desired_intent()
                .unwrap()
                .destination
                .camera
                .position,
            destination_b.camera.position
        );
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);

        commit_current(&mut controller);
        assert_eq!(
            controller.active_presentation().view.camera.position,
            destination_b.camera.position
        );
        assert_eq!(position(&controller), body_a);
        assert_invariants(&controller);
    }

    #[test]
    fn staging_and_prepared_tokens_match_every_intent_identity_field() {
        let mut controller = controller();
        controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 2),
                contract: contract(7, 9),
            })
            .unwrap();
        let token = controller.desired_intent().unwrap().staging_token();

        let mutations = [
            StagingToken {
                intent: IntentId(token.intent.0 + 1),
                ..token
            },
            StagingToken {
                source_generation: PresentationGeneration(token.source_generation.0 + 1),
                ..token
            },
            StagingToken {
                destination_owner: ViewOwner::Profile,
                ..token
            },
            StagingToken {
                destination_scene: SceneToken(token.destination_scene.0 + 1),
                ..token
            },
            StagingToken {
                destination_presentation: PresentationToken(token.destination_presentation.0 + 1),
                ..token
            },
            StagingToken {
                contract: PresentationContract {
                    identity: ContractIdentity(token.contract.identity.0 + 1),
                    ..token.contract
                },
                ..token
            },
            StagingToken {
                contract: PresentationContract {
                    revision: ContractRevision(token.contract.revision.0 + 1),
                    ..token.contract
                },
                ..token
            },
        ];
        for mutation in mutations {
            assert_eq!(
                controller.prepare_completion(mutation),
                Err(ViewError::StaleCompletion)
            );
        }
        assert!(controller.prepare_completion(token).is_ok());
        assert_invariants(&controller);
    }

    #[test]
    fn stale_b_completion_is_rejected_after_c_supersedes_it() {
        let mut controller = controller();
        controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 2),
                contract: contract(100, 2),
            })
            .unwrap();
        let intent_b = controller.desired_intent().unwrap();
        let prepared_b = prepare_current(&controller);

        controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 3),
                contract: contract(1, 3),
            })
            .unwrap();
        let intent_c = controller.desired_intent().unwrap();
        assert!(intent_c.id > intent_b.id);
        assert_eq!(intent_c.source_generation, intent_b.source_generation);
        assert_ne!(
            intent_c.destination.presentation, intent_b.destination.presentation,
            "the controller mints a fresh presentation identity for every changed target"
        );

        let active_a = controller.active_presentation();
        assert_eq!(
            controller.reduce(ViewEvent::Commit(prepared_b)),
            Err(ViewError::StaleCompletion)
        );
        assert_eq!(
            controller.active_presentation().view.presentation,
            active_a.view.presentation
        );
        assert_eq!(controller.desired_intent().unwrap().id, intent_c.id);

        commit_current(&mut controller);
        assert_eq!(
            controller.active_presentation().view.presentation,
            intent_c.destination.presentation
        );
        assert_invariants(&controller);
    }

    #[test]
    fn movement_requires_a_one_use_exact_active_contract_admission() {
        let mut controller = controller();
        assert!(matches!(
            controller.admit_movement(camera(2.0), |_, _| false),
            Err(ViewError::MovementOutsideContract)
        ));
        let stale_after_commit = admit(&controller, camera(3.0));

        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 4),
                contract: contract(1, 1),
            })
            .unwrap();
        commit_current(&mut controller);
        assert_eq!(
            controller.reduce(ViewEvent::UpdateSpectatorView(stale_after_commit)),
            Err(ViewError::StaleMovementAdmission)
        );

        let active_contract = controller.active_presentation().contract;
        assert_eq!(
            controller
                .reduce(ViewEvent::ContractInvalidated(active_contract))
                .unwrap(),
            ReducerEffect::AuthorityRevoked
        );
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        assert!(matches!(
            controller.admit_movement(camera(5.0), |_, _| true),
            Err(ViewError::AuthorityDenied)
        ));
        assert_invariants(&controller);
    }

    #[test]
    fn invalidation_rejects_prepared_work_but_stale_invalidation_cannot_revoke_fresh_work() {
        let mut controller = controller();
        let invalidated = contract(1, 1);
        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 2),
                contract: invalidated,
            })
            .unwrap();
        let prepared = prepare_current(&controller);
        assert_eq!(
            controller
                .reduce(ViewEvent::ContractInvalidated(invalidated))
                .unwrap(),
            ReducerEffect::Unchanged
        );
        assert_eq!(
            controller.reduce(ViewEvent::Commit(prepared)),
            Err(ViewError::StaleCompletion)
        );
        assert_eq!(
            controller.prepare_completion(
                controller
                    .desired_intent()
                    .expect("invalidated intent remains inspectable")
                    .staging_token()
            ),
            Err(ViewError::StaleCompletion)
        );
        controller
            .reduce(ViewEvent::SpectatorCapabilityLost {
                destination: gameplay_view(20),
                contract: contract(100, 2),
            })
            .unwrap();

        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 3),
                contract: contract(1, 2),
            })
            .unwrap();
        commit_current(&mut controller);
        controller
            .reduce(ViewEvent::ExitSpectator {
                destination: gameplay_view(21),
                contract: contract(100, 3),
            })
            .unwrap();
        commit_current(&mut controller);
        assert_eq!(controller.authority_lease(), AuthorityLease::Gameplay);
        assert_eq!(
            controller
                .reduce(ViewEvent::ContractInvalidated(contract(100, 1)))
                .unwrap(),
            ReducerEffect::Unchanged
        );
        assert_eq!(controller.authority_lease(), AuthorityLease::Gameplay);
        assert_invariants(&controller);
    }

    #[test]
    fn unrelated_invalidation_cannot_revive_an_invalid_active_contract() {
        let mut controller = controller();
        let active_a = controller.active_presentation().contract;
        let unrelated_b = contract(999, 1);

        assert_eq!(
            controller
                .reduce(ViewEvent::ContractInvalidated(active_a))
                .unwrap(),
            ReducerEffect::AuthorityRevoked
        );
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        assert_eq!(
            controller
                .reduce(ViewEvent::ContractInvalidated(unrelated_b))
                .unwrap(),
            ReducerEffect::Unchanged
        );
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        assert_invariants(&controller);

        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 2),
                contract: contract(1, 2),
            })
            .unwrap();
        commit_current(&mut controller);
        assert_eq!(controller.authority_lease(), AuthorityLease::SpectatorView);
        assert_invariants(&controller);
    }

    #[test]
    fn authority_lease_is_the_only_body_and_view_mutation_gate() {
        let mut controller = controller();
        assert_eq!(controller.authority_lease(), AuthorityLease::Gameplay);
        assert!(controller.authority_lease().allows_body_simulation());
        assert!(controller.authority_lease().allows_edits());
        assert!(!controller.authority_lease().allows_spectator_view_input());
        let update = admit(&controller, camera(2.0));
        controller
            .reduce(ViewEvent::UpdateGameplayBody(update))
            .unwrap();
        assert_eq!(position(&controller), camera(2.0).position);

        let denied_body = admit(&controller, camera(4.0));
        let denied_view = admit(&controller, camera(4.0));
        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 3),
                contract: contract(1, 1),
            })
            .unwrap();
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        assert_eq!(
            controller.reduce(ViewEvent::UpdateGameplayBody(denied_body)),
            Err(ViewError::AuthorityDenied)
        );
        assert_eq!(
            controller.reduce(ViewEvent::UpdateSpectatorView(denied_view)),
            Err(ViewError::AuthorityDenied)
        );

        commit_current(&mut controller);
        assert_eq!(controller.authority_lease(), AuthorityLease::SpectatorView);
        assert!(controller.authority_lease().allows_spectator_view_input());
        assert!(!controller.authority_lease().allows_body_simulation());
        assert!(!controller.authority_lease().allows_edits());
        let spectator_update = admit(&controller, camera(5.0));
        controller
            .reduce(ViewEvent::UpdateSpectatorView(spectator_update))
            .unwrap();
        assert_eq!(
            controller.active_presentation().view.camera.position,
            camera(5.0).position
        );
        let gameplay_from_spectator = admit(&controller, camera(6.0));
        assert_eq!(
            controller.reduce(ViewEvent::UpdateGameplayBody(gameplay_from_spectator)),
            Err(ViewError::AuthorityDenied)
        );
        assert_eq!(position(&controller), camera(2.0).position);
        assert_invariants(&controller);
    }

    #[test]
    fn spectator_exit_reenter_and_capability_loss_preserve_the_body() {
        let mut controller = controller();
        let body = position(&controller);
        controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 2),
                contract: contract(1, 1),
            })
            .unwrap();
        commit_current(&mut controller);
        let spectator_update = admit(&controller, camera(3.0));
        controller
            .reduce(ViewEvent::UpdateSpectatorView(spectator_update))
            .unwrap();

        controller
            .reduce(ViewEvent::ExitSpectator {
                destination: gameplay_view(20),
                contract: contract(100, 2),
            })
            .unwrap();
        let exit = controller.desired_intent().unwrap();
        assert_eq!(exit.destination.camera.position, body);
        assert_eq!(
            controller.active_presentation().view.camera.position,
            camera(3.0).position
        );

        let ReducerEffect::IntentCancelled(cancelled) = controller
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 4),
                contract: contract(1, 3),
            })
            .unwrap()
        else {
            panic!("re-enter must cancel the pending exit");
        };
        assert_eq!(cancelled, exit.id);
        assert!(controller.desired_intent().is_none());
        assert!(matches!(controller.session(), ViewSession::Spectator));

        controller
            .reduce(ViewEvent::SpectatorCapabilityLost {
                destination: gameplay_view(21),
                contract: contract(100, 4),
            })
            .unwrap();
        assert_eq!(
            controller
                .desired_intent()
                .unwrap()
                .destination
                .camera
                .position,
            body
        );
        let forced = controller.desired_intent().unwrap().id;
        assert_eq!(
            controller
                .reduce(ViewEvent::EnterSpectator {
                    destination: view(ViewOwner::Spectator, 5),
                    contract: contract(1, 5),
                })
                .unwrap(),
            ReducerEffect::IntentCancelled(forced),
            "a fresh explicit enter after capability restoration cancels the forced return"
        );
        assert!(controller.desired_intent().is_none());
        controller
            .reduce(ViewEvent::SpectatorCapabilityLost {
                destination: gameplay_view(22),
                contract: contract(100, 6),
            })
            .unwrap();
        commit_current(&mut controller);
        assert!(matches!(controller.session(), ViewSession::Gameplay));
        assert_eq!(controller.active_presentation().view.camera.position, body);
        assert_eq!(position(&controller), body);
        assert_invariants(&controller);
    }

    #[test]
    fn identical_level_triggered_requests_do_not_starve_the_current_intent() {
        let mut spectator = controller();
        spectator
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 2),
                contract: contract(1, 1),
            })
            .unwrap();
        commit_current(&mut spectator);
        let forced_destination = gameplay_view(20);
        let forced_contract = contract(100, 2);
        spectator
            .reduce(ViewEvent::SpectatorCapabilityLost {
                destination: forced_destination,
                contract: forced_contract,
            })
            .unwrap();
        let forced = spectator.desired_intent().unwrap();
        assert_eq!(
            spectator
                .reduce(ViewEvent::SpectatorCapabilityLost {
                    destination: forced_destination,
                    contract: forced_contract,
                })
                .unwrap(),
            ReducerEffect::Unchanged
        );
        assert_eq!(spectator.desired_intent().unwrap().id, forced.id);

        let mut reproduction = controller();
        let capture = view(ViewOwner::Reproduction, 3);
        let apply_contract = contract(3, 1);
        reproduction
            .reduce(ViewEvent::ApplyReproduction {
                destination: capture,
                contract: apply_contract,
            })
            .unwrap();
        let apply = reproduction.desired_intent().unwrap();
        assert_eq!(
            reproduction
                .reduce(ViewEvent::ApplyReproduction {
                    destination: capture,
                    contract: apply_contract,
                })
                .unwrap(),
            ReducerEffect::Unchanged
        );
        assert_eq!(reproduction.desired_intent().unwrap().id, apply.id);
        commit_current(&mut reproduction);

        let clear_destination = gameplay_view(21);
        let clear_contract = contract(100, 3);
        reproduction
            .reduce(ViewEvent::ClearReproduction {
                destination: clear_destination,
                contract: clear_contract,
            })
            .unwrap();
        let clear = reproduction.desired_intent().unwrap();
        assert_eq!(
            reproduction
                .reduce(ViewEvent::ClearReproduction {
                    destination: clear_destination,
                    contract: clear_contract,
                })
                .unwrap(),
            ReducerEffect::Unchanged
        );
        assert_eq!(reproduction.desired_intent().unwrap().id, clear.id);
    }

    #[test]
    fn profile_ticks_advance_exactly_once_and_terminal_submission_gates_return() {
        let mut controller = controller();
        let body = position(&controller);
        controller
            .reduce(ViewEvent::StartProfile {
                destination: view(ViewOwner::Profile, 2),
                contract: contract(2, 1),
            })
            .unwrap();
        commit_current(&mut controller);
        let ViewSession::Profile(profile) = controller.session() else {
            panic!("profile start must commit its session");
        };
        assert_eq!(profile.committed_ticks(), 0);
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);

        controller
            .reduce(ViewEvent::ProfileTick {
                tick: 1,
                destination: view(ViewOwner::Profile, 3),
                terminal: false,
                contract: contract(2, 2),
            })
            .unwrap();
        let prepared_tick_one = prepare_current(&controller);
        let ViewSession::Profile(profile) = controller.session() else {
            unreachable!();
        };
        assert_eq!(profile.committed_ticks(), 0);
        controller
            .reduce(ViewEvent::Commit(prepared_tick_one))
            .unwrap();
        let ViewSession::Profile(profile) = controller.session() else {
            unreachable!();
        };
        assert_eq!(profile.committed_ticks(), 1);
        assert_eq!(
            controller.reduce(ViewEvent::Commit(prepared_tick_one)),
            Err(ViewError::NoTransitionInFlight)
        );
        assert_eq!(
            controller.reduce(ViewEvent::ProfileTick {
                tick: 1,
                destination: view(ViewOwner::Profile, 4),
                terminal: false,
                contract: contract(2, 3),
            }),
            Err(ViewError::WrongProfileTick {
                expected: 2,
                actual: 1
            })
        );

        controller
            .reduce(ViewEvent::ProfileTick {
                tick: 2,
                destination: view(ViewOwner::Profile, 5),
                terminal: true,
                contract: contract(2, 4),
            })
            .unwrap();
        let terminal_generation = commit_current(&mut controller);
        let ViewSession::Profile(profile) = controller.session() else {
            unreachable!();
        };
        assert_eq!(profile.committed_ticks(), 2);
        assert_eq!(
            profile.phase(),
            ProfileSessionPhase::Drain {
                terminal_generation,
                frame_submitted: false
            }
        );
        assert!(controller.desired_intent().is_none());
        assert_eq!(
            controller.reduce(ViewEvent::ProfileTerminalFrameSubmitted {
                generation: PresentationGeneration(terminal_generation.0 + 1),
                destination: gameplay_view(20),
                contract: contract(100, 5),
            }),
            Err(ViewError::WrongTerminalGeneration {
                expected: terminal_generation,
                actual: PresentationGeneration(terminal_generation.0 + 1)
            })
        );
        assert!(controller.desired_intent().is_none());

        controller
            .reduce(ViewEvent::ProfileTerminalFrameSubmitted {
                generation: terminal_generation,
                destination: gameplay_view(21),
                contract: contract(100, 5),
            })
            .unwrap();
        let ViewSession::Profile(profile) = controller.session() else {
            unreachable!();
        };
        assert_eq!(
            profile.phase(),
            ProfileSessionPhase::Drain {
                terminal_generation,
                frame_submitted: true
            }
        );
        assert_eq!(
            controller
                .desired_intent()
                .unwrap()
                .destination
                .camera
                .position,
            body
        );
        commit_current(&mut controller);
        assert!(matches!(controller.session(), ViewSession::Gameplay));
        assert_eq!(position(&controller), body);
        assert_invariants(&controller);
    }

    #[test]
    fn profile_cancel_returns_to_a_fresh_gameplay_contract_and_stales_pending_ticks() {
        let mut controller = controller();
        let body = position(&controller);
        controller
            .reduce(ViewEvent::StartProfile {
                destination: view(ViewOwner::Profile, 2),
                contract: contract(2, 1),
            })
            .unwrap();
        let start_id = controller.desired_intent().unwrap().id;
        assert_eq!(
            controller
                .reduce(ViewEvent::ProfileCancel {
                    destination: gameplay_view(20),
                    contract: contract(100, 2),
                })
                .unwrap(),
            ReducerEffect::IntentCancelled(start_id)
        );

        controller
            .reduce(ViewEvent::StartProfile {
                destination: view(ViewOwner::Profile, 3),
                contract: contract(2, 3),
            })
            .unwrap();
        commit_current(&mut controller);
        controller
            .reduce(ViewEvent::ProfileTick {
                tick: 1,
                destination: view(ViewOwner::Profile, 4),
                terminal: false,
                contract: contract(2, 4),
            })
            .unwrap();
        let stale_tick = prepare_current(&controller);
        let tick_id = controller.desired_intent().unwrap().id;
        controller
            .reduce(ViewEvent::ProfileCancel {
                destination: gameplay_view(21),
                contract: contract(100, 5),
            })
            .unwrap();
        let cancel = controller.desired_intent().unwrap();
        assert!(cancel.id > tick_id);
        assert_ne!(
            cancel.destination.presentation,
            controller.active_presentation().view.presentation
        );
        assert_eq!(
            cancel.destination.scene,
            controller.authoritative_body().scene()
        );
        assert_eq!(cancel.destination.camera.position, body);
        assert_eq!(
            controller.reduce(ViewEvent::Commit(stale_tick)),
            Err(ViewError::StaleCompletion)
        );
        commit_current(&mut controller);
        assert!(matches!(controller.session(), ViewSession::Gameplay));
        assert_eq!(controller.active_presentation().view.camera.position, body);
        assert_invariants(&controller);
    }

    #[test]
    fn reproduction_apply_replace_clear_cycles_never_lose_the_stable_base() {
        let mut controller = controller();
        let body = position(&controller);

        let ReducerEffect::IntentStarted(first) = controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 2),
                contract: contract(3, 1),
            })
            .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(
            controller
                .reduce(ViewEvent::ClearReproduction {
                    destination: gameplay_view(20),
                    contract: contract(100, 2)
                })
                .unwrap(),
            ReducerEffect::IntentCancelled(first)
        );
        assert!(matches!(controller.session(), ViewSession::Gameplay));
        assert!(controller.desired_intent().is_none());

        controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 3),
                contract: contract(3, 3),
            })
            .unwrap();
        commit_current(&mut controller);
        assert!(matches!(controller.session(), ViewSession::Reproduction));
        assert_eq!(controller.authority_lease(), AuthorityLease::Denied);
        assert_eq!(
            controller.reduce(ViewEvent::ClearReproduction {
                destination: view(ViewOwner::Gameplay, 99),
                contract: contract(100, 4),
            }),
            Err(ViewError::WrongGameplayScene {
                expected: SceneToken(11),
                actual: SceneToken(991),
            })
        );
        assert_eq!(
            controller.reduce(ViewEvent::ClearReproduction {
                destination: gameplay_view(99),
                contract: contract(999, 4),
            }),
            Err(ViewError::WrongGameplayContractIdentity {
                expected: ContractIdentity(100),
                actual: ContractIdentity(999),
            })
        );

        controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 4),
                contract: contract(3, 4),
            })
            .unwrap();
        let stale_replace = prepare_current(&controller);
        let replace_id = controller.desired_intent().unwrap().id;
        controller
            .reduce(ViewEvent::ClearReproduction {
                destination: gameplay_view(21),
                contract: contract(100, 5),
            })
            .unwrap();
        let clear_id = controller.desired_intent().unwrap().id;
        assert!(clear_id > replace_id);
        assert_eq!(
            controller.reduce(ViewEvent::Commit(stale_replace)),
            Err(ViewError::StaleCompletion)
        );
        let clear = controller.desired_intent().unwrap();
        assert_ne!(
            clear.destination.presentation,
            controller.active_presentation().view.presentation
        );
        assert_eq!(
            clear.destination.scene,
            controller.authoritative_body().scene()
        );

        controller
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 5),
                contract: contract(3, 6),
            })
            .unwrap();
        assert!(controller.desired_intent().unwrap().id > clear_id);
        commit_current(&mut controller);
        assert!(matches!(controller.session(), ViewSession::Reproduction));

        controller
            .reduce(ViewEvent::ClearReproduction {
                destination: gameplay_view(22),
                contract: contract(100, 7),
            })
            .unwrap();
        let clear = controller.desired_intent().unwrap();
        assert_ne!(
            clear.destination.presentation,
            controller.active_presentation().view.presentation
        );
        assert_eq!(
            clear.destination.scene,
            controller.authoritative_body().scene()
        );
        commit_current(&mut controller);
        assert!(matches!(controller.session(), ViewSession::Gameplay));
        assert_eq!(controller.active_presentation().view.camera.position, body);
        assert_eq!(position(&controller), body);
        assert_invariants(&controller);
    }

    #[test]
    fn shutdown_always_returns_the_durable_body_in_every_owner_and_transition() {
        let mut owner = controller();
        let body = position(&owner);
        assert_eq!(owner.shutdown_camera().position, body);

        owner
            .reduce(ViewEvent::EnterSpectator {
                destination: view(ViewOwner::Spectator, 2),
                contract: contract(4, 1),
            })
            .unwrap();
        assert_eq!(owner.shutdown_camera().position, body);
        commit_current(&mut owner);
        assert_eq!(owner.shutdown_camera().position, body);
        owner
            .reduce(ViewEvent::ExitSpectator {
                destination: gameplay_view(20),
                contract: contract(100, 2),
            })
            .unwrap();
        assert_eq!(owner.shutdown_camera().position, body);
        commit_current(&mut owner);

        owner
            .reduce(ViewEvent::StartProfile {
                destination: view(ViewOwner::Profile, 3),
                contract: contract(4, 3),
            })
            .unwrap();
        assert_eq!(owner.shutdown_camera().position, body);
        commit_current(&mut owner);
        assert_eq!(owner.shutdown_camera().position, body);

        let mut reproduction = controller();
        reproduction
            .reduce(ViewEvent::ApplyReproduction {
                destination: view(ViewOwner::Reproduction, 4),
                contract: contract(4, 4),
            })
            .unwrap();
        assert_eq!(reproduction.shutdown_camera().position, body);
        commit_current(&mut reproduction);
        assert_eq!(reproduction.shutdown_camera().position, body);
        reproduction
            .reduce(ViewEvent::ClearReproduction {
                destination: gameplay_view(21),
                contract: contract(100, 5),
            })
            .unwrap();
        assert_eq!(reproduction.shutdown_camera().position, body);
    }

    #[derive(Clone, Copy)]
    struct Deterministic(u64);

    impl Deterministic {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }
    }

    #[test]
    fn generated_event_sequences_preserve_body_and_transaction_invariants() {
        for seed in 1..=96 {
            let mut random = Deterministic(seed);
            let mut controller = controller();
            let mut prepared = Vec::new();
            let mut greatest_intent = IntentId(0);
            let mut token_marker = 10u64;

            for _ in 0..400 {
                token_marker = token_marker.saturating_add(1);
                let choice = random.next() % 14;
                let before_body = position(&controller);
                let before_lease = controller.authority_lease();
                let body_update = choice == 0;
                let event = match choice {
                    0 => {
                        let Ok(admission) =
                            controller.admit_movement(camera(token_marker as f32), |_, _| true)
                        else {
                            assert_invariants(&controller);
                            continue;
                        };
                        ViewEvent::UpdateGameplayBody(admission)
                    }
                    1 => {
                        let Ok(admission) =
                            controller.admit_movement(camera(token_marker as f32), |_, _| true)
                        else {
                            assert_invariants(&controller);
                            continue;
                        };
                        ViewEvent::UpdateSpectatorView(admission)
                    }
                    2 => ViewEvent::EnterSpectator {
                        destination: view(ViewOwner::Spectator, token_marker),
                        contract: contract(100, random.next()),
                    },
                    3 => ViewEvent::ExitSpectator {
                        destination: gameplay_view(token_marker),
                        contract: contract(100, random.next()),
                    },
                    4 => ViewEvent::SpectatorCapabilityLost {
                        destination: gameplay_view(token_marker),
                        contract: contract(100, random.next()),
                    },
                    5 => ViewEvent::StartProfile {
                        destination: view(ViewOwner::Profile, token_marker),
                        contract: contract(100, random.next()),
                    },
                    6 => {
                        let tick = match controller.session() {
                            ViewSession::Profile(profile) => profile
                                .committed_ticks()
                                .saturating_add(1 + (random.next() % 3)),
                            _ => random.next() % 8,
                        };
                        ViewEvent::ProfileTick {
                            tick,
                            destination: view(ViewOwner::Profile, token_marker),
                            terminal: random.next().is_multiple_of(5),
                            contract: contract(20, random.next()),
                        }
                    }
                    7 => ViewEvent::ProfileTerminalFrameSubmitted {
                        generation: if random.next().is_multiple_of(2) {
                            controller.active_presentation().generation
                        } else {
                            PresentationGeneration(random.next())
                        },
                        destination: gameplay_view(token_marker),
                        contract: contract(100, random.next()),
                    },
                    8 => ViewEvent::ProfileCancel {
                        destination: gameplay_view(token_marker),
                        contract: contract(100, random.next()),
                    },
                    9 => ViewEvent::ApplyReproduction {
                        destination: view(ViewOwner::Reproduction, token_marker),
                        contract: contract(100, random.next()),
                    },
                    10 => ViewEvent::ClearReproduction {
                        destination: gameplay_view(token_marker),
                        contract: contract(100, random.next()),
                    },
                    11 => ViewEvent::ContractInvalidated(if random.next().is_multiple_of(2) {
                        controller.active_presentation().contract
                    } else {
                        contract(random.next(), random.next())
                    }),
                    _ => {
                        if let Some(desired) = controller.desired_intent()
                            && let Ok(completion) =
                                controller.prepare_completion(desired.staging_token())
                        {
                            prepared.push(completion);
                        }
                        let completion = if random.next().is_multiple_of(2) {
                            prepared.last().copied()
                        } else if prepared.is_empty() {
                            None
                        } else {
                            Some(prepared[(random.next() as usize) % prepared.len()])
                        };
                        let Some(completion) = completion else {
                            assert_invariants(&controller);
                            continue;
                        };
                        ViewEvent::Commit(completion)
                    }
                };

                let result = controller.reduce(event);
                let after_body = position(&controller);
                if after_body != before_body {
                    assert!(body_update);
                    assert_eq!(before_lease, AuthorityLease::Gameplay);
                    assert!(result.is_ok());
                }
                if let Some(desired) = controller.desired_intent()
                    && desired.id != greatest_intent
                {
                    assert!(desired.id > greatest_intent);
                    greatest_intent = desired.id;
                }
                assert_invariants(&controller);
            }
        }
    }
}
