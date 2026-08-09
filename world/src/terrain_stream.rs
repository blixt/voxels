//! Bounded deadline scheduling and content-addressed caching for virtual terrain pages.
//!
//! The renderer reports exact missing page identities. This module turns those demands into
//! complete replacement-group requests without tying policy to a browser, transport, or GPU. All
//! queues have explicit ceilings; pages already in flight may become obsolete, but pending work is
//! cancelled immediately and late bytes are measured rather than silently admitted as useful.

use crate::{
    TERRAIN_PAGE_MAX_CHILDREN, TERRAIN_PAGE_MAX_COMPRESSED_BYTES, TERRAIN_PAGE_TRANSFER_MAX_BYTES,
    TERRAIN_PAGE_TRANSFER_MAX_ITEMS, TerrainPageKey, TerrainPageTransferIdentity, TerrainPageV1,
    WorldSourceIdentityHash, decode_terrain_page,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainStreamConfig {
    pub max_pending_pages: usize,
    pub max_in_flight_pages: usize,
    pub max_batch_items: usize,
    pub max_batch_bytes: usize,
    pub retry_base_ms: u64,
    pub retry_max_ms: u64,
}

impl TerrainStreamConfig {
    pub const DEVELOPMENT: Self = Self {
        max_pending_pages: 2_048,
        max_in_flight_pages: 256,
        max_batch_items: 64,
        max_batch_bytes: 2 * 1_024 * 1_024,
        retry_base_ms: 100,
        retry_max_ms: 5_000,
    };

    /// Browser-facing pacing that keeps decode and GPU publication inside a 120 Hz frame budget.
    ///
    /// Eight pages cover the widest atomic replacement group: volumetric octree children. Surface
    /// quadtree replacements remain four pages, but the interactive scheduler must not admit a
    /// valid group it can never dispatch.
    pub const INTERACTIVE_CLIENT: Self = Self {
        max_pending_pages: 2_048,
        max_in_flight_pages: 64,
        max_batch_items: TERRAIN_PAGE_MAX_CHILDREN,
        max_batch_bytes: 2 * 1_024 * 1_024,
        retry_base_ms: 100,
        retry_max_ms: 5_000,
    };

    pub fn validates(self) -> bool {
        self.max_pending_pages >= TERRAIN_PAGE_MAX_CHILDREN
            && self.max_in_flight_pages > 0
            && self.max_batch_items > 0
            && self.max_batch_items <= TERRAIN_PAGE_TRANSFER_MAX_ITEMS
            && self.max_batch_items <= self.max_in_flight_pages
            && self.max_batch_bytes >= TERRAIN_PAGE_MAX_COMPRESSED_BYTES
            && self.max_batch_bytes <= TERRAIN_PAGE_TRANSFER_MAX_BYTES
            && self.retry_base_ms > 0
            && self.retry_base_ms <= self.retry_max_ms
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainPageDemand {
    pub identity: TerrainPageTransferIdentity,
    pub projected_error_millipixels: u32,
    pub time_to_exposure_ms: u32,
    pub distance_millimetres: u32,
    /// Zero means certainly visible; 1,000 means confidently occluded.
    pub occlusion_confidence_millis: u16,
    /// This page is on the dependency chain of the next exact playable presentation envelope.
    ///
    /// Unlike visual topology or silhouette error, this is a movement-safety dependency: optional
    /// refinement must never consume the stream indefinitely while this work is pending.
    pub presentation_critical: bool,
    pub topology_critical: bool,
    pub silhouette_critical: bool,
    pub estimated_encoded_bytes: u32,
}

impl TerrainPageDemand {
    pub fn validates(self) -> bool {
        self.identity.key.level <= crate::TERRAIN_PAGE_MAX_LEVEL
            && self.identity.content_fingerprint != [0; 32]
            && self.occlusion_confidence_millis <= 1_000
            && self.estimated_encoded_bytes > 0
            && self.estimated_encoded_bytes as usize <= TERRAIN_PAGE_MAX_COMPRESSED_BYTES
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainDemandGroup {
    /// `Some(parent)` means all eight exact children form one atomic replacement unit.
    pub replacement_parent: Option<TerrainPageKey>,
    pub pages: Vec<TerrainPageDemand>,
}

/// Returns every complete refinement replacement represented by `identities`.
///
/// This is shared by admission, cache hydration, and GPU publication so all three stages use the
/// same atomicity boundary. An incomplete sibling set is deliberately omitted: callers may cache
/// its pages, but must not expose them as a partial hierarchy replacement.
pub fn terrain_page_replacement_groups(
    identities: &[TerrainPageTransferIdentity],
) -> Vec<(TerrainPageKey, Vec<TerrainPageTransferIdentity>)> {
    let requested = identities
        .iter()
        .map(|identity| (identity.key, *identity))
        .collect::<BTreeMap<_, _>>();
    requested
        .keys()
        .filter_map(|key| key.parent())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|parent| {
            let children = parent.refinement_children()?;
            let group = children
                .iter()
                .map(|child| requested.get(child).copied())
                .collect::<Option<Vec<_>>>()?;
            Some((parent, group))
        })
        .collect()
}

impl TerrainDemandGroup {
    pub fn singleton(page: TerrainPageDemand) -> Self {
        Self {
            replacement_parent: None,
            pages: vec![page],
        }
    }

    pub fn replacement(
        parent: TerrainPageKey,
        pages: Vec<TerrainPageDemand>,
    ) -> Result<Self, TerrainStreamError> {
        let group = Self {
            replacement_parent: Some(parent),
            pages,
        };
        if !group.validates() {
            return Err(TerrainStreamError::InvalidDemandGroup);
        }
        Ok(group)
    }

    /// The canonical ordering shared by transport and cache-to-GPU admission.
    pub fn compare_priority(&self, other: &Self) -> Ordering {
        compare_demand_groups(self, other)
    }

    fn validates(&self) -> bool {
        if self.pages.is_empty()
            || self.pages.len() > TERRAIN_PAGE_MAX_CHILDREN
            || self.pages.iter().any(|page| !page.validates())
        {
            return false;
        }
        let identities = self
            .pages
            .iter()
            .map(|page| page.identity)
            .collect::<BTreeSet<_>>();
        if identities.len() != self.pages.len() {
            return false;
        }
        let Some(parent) = self.replacement_parent else {
            return self.pages.len() == 1;
        };
        parent.refinement_children().is_some_and(|children| {
            self.pages.len() == children.len()
                && children
                    .into_iter()
                    .all(|child| self.pages.iter().any(|page| page.identity.key == child))
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerrainStreamStats {
    pub pending_pages: usize,
    pub in_flight_pages: usize,
    pub cancelled_pending_pages: u64,
    pub obsolete_in_flight_pages: usize,
    pub useful_bytes: u64,
    pub cancellation_waste_bytes: u64,
    pub failed_pages: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingDemand {
    demand: TerrainPageDemand,
    replacement_parent: Option<TerrainPageKey>,
    attempt: u8,
    retry_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InFlightDemand {
    pending: PendingDemand,
    obsolete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainRequestBatch {
    pages: Vec<TerrainPageTransferIdentity>,
}

impl TerrainRequestBatch {
    pub fn pages(&self) -> &[TerrainPageTransferIdentity] {
        &self.pages
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainStreamError {
    InvalidConfig,
    InvalidDemandGroup,
    InvalidRequestBatch,
    UnschedulableDemandGroup,
    UnknownPending(TerrainPageTransferIdentity),
    UnknownInFlight(TerrainPageTransferIdentity),
}

impl fmt::Display for TerrainStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid virtual terrain stream capacity"),
            Self::InvalidDemandGroup => {
                formatter.write_str("invalid or incomplete virtual terrain demand group")
            }
            Self::InvalidRequestBatch => {
                formatter.write_str("invalid or duplicate virtual terrain request batch")
            }
            Self::UnschedulableDemandGroup => {
                formatter.write_str("virtual terrain demand group exceeds stream batch capacity")
            }
            Self::UnknownPending(identity) => {
                write!(
                    formatter,
                    "virtual terrain page {identity:?} is not pending"
                )
            }
            Self::UnknownInFlight(identity) => {
                write!(
                    formatter,
                    "virtual terrain page {identity:?} is not in flight"
                )
            }
        }
    }
}

impl std::error::Error for TerrainStreamError {}

pub struct TerrainStreamScheduler {
    config: TerrainStreamConfig,
    pending: BTreeMap<TerrainPageTransferIdentity, PendingDemand>,
    in_flight: BTreeMap<TerrainPageTransferIdentity, InFlightDemand>,
    cancelled_pending_pages: u64,
    useful_bytes: u64,
    cancellation_waste_bytes: u64,
    failed_pages: u64,
}

impl TerrainStreamScheduler {
    pub fn new(config: TerrainStreamConfig) -> Result<Self, TerrainStreamError> {
        if !config.validates() {
            return Err(TerrainStreamError::InvalidConfig);
        }
        Ok(Self {
            config,
            pending: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            cancelled_pending_pages: 0,
            useful_bytes: 0,
            cancellation_waste_bytes: 0,
            failed_pages: 0,
        })
    }

    pub const fn config(&self) -> TerrainStreamConfig {
        self.config
    }

    /// Replaces pending demand with the best complete groups for the current predicted view.
    ///
    /// A group is either admitted whole or omitted whole. Already in-flight work is not duplicated;
    /// if it leaves the demand set it is marked obsolete so a late response can be measured.
    pub fn reconcile(
        &mut self,
        groups: impl IntoIterator<Item = TerrainDemandGroup>,
    ) -> Result<(), TerrainStreamError> {
        self.reconcile_with_available(groups, std::iter::empty())
    }

    /// Reconciles complete logical demand groups while omitting identities already available in
    /// the encoded cache.
    ///
    /// A partially cached sibling group remains one topology dependency, but only its missing
    /// members are pending transport work. Publication still waits for the complete group in the
    /// cache; successful siblings are never retransmitted merely to preserve request batching.
    pub fn reconcile_with_available(
        &mut self,
        groups: impl IntoIterator<Item = TerrainDemandGroup>,
        available: impl IntoIterator<Item = TerrainPageTransferIdentity>,
    ) -> Result<(), TerrainStreamError> {
        let available = available.into_iter().collect::<BTreeSet<_>>();
        let mut groups = groups.into_iter().collect::<Vec<_>>();
        if groups.iter().any(|group| !group.validates()) {
            return Err(TerrainStreamError::InvalidDemandGroup);
        }
        if groups.iter().any(|group| {
            let mut missing_pages = 0usize;
            let mut missing_bytes = 0usize;
            for page in &group.pages {
                if available.contains(&page.identity) || self.in_flight.contains_key(&page.identity)
                {
                    continue;
                }
                missing_pages = missing_pages.saturating_add(1);
                missing_bytes = missing_bytes.saturating_add(page.estimated_encoded_bytes as usize);
            }
            missing_pages > self.config.max_batch_items
                || missing_pages > self.config.max_in_flight_pages
                || missing_bytes > self.config.max_batch_bytes
        }) {
            return Err(TerrainStreamError::UnschedulableDemandGroup);
        }
        let demanded_missing = groups
            .iter()
            .flat_map(|group| group.pages.iter().map(|page| page.identity))
            .filter(|identity| !available.contains(identity))
            .collect::<BTreeSet<_>>();
        groups.sort_by(compare_demand_groups);
        let mut admitted = BTreeMap::new();
        for group in groups {
            let new_pages = group
                .pages
                .iter()
                .filter(|page| {
                    !available.contains(&page.identity)
                        && !self.in_flight.contains_key(&page.identity)
                })
                .count();
            if admitted.len().saturating_add(new_pages) > self.config.max_pending_pages {
                continue;
            }
            for demand in group.pages {
                if available.contains(&demand.identity)
                    || self.in_flight.contains_key(&demand.identity)
                {
                    continue;
                }
                let prior = self.pending.get(&demand.identity);
                admitted.insert(
                    demand.identity,
                    PendingDemand {
                        demand,
                        replacement_parent: group.replacement_parent,
                        attempt: prior.map_or(0, |pending| pending.attempt),
                        retry_at_ms: prior.map_or(0, |pending| pending.retry_at_ms),
                    },
                );
            }
        }
        self.cancelled_pending_pages = self.cancelled_pending_pages.saturating_add(
            self.pending
                .keys()
                .filter(|key| !admitted.contains_key(key))
                .count() as u64,
        );
        self.pending = admitted;
        for (identity, entry) in &mut self.in_flight {
            entry.obsolete = !demanded_missing.contains(identity);
        }
        Ok(())
    }

    /// Selects the next atomic transport batch without moving it to in-flight.
    ///
    /// When siblings already exist in the encoded cache this contains only the missing members of
    /// their complete logical replacement group.
    ///
    /// Transport submission is the capability boundary. Callers commit only after the transport
    /// has accepted the batch, so window pressure and socket backpressure cannot manufacture
    /// failed in-flight work.
    pub fn peek_batch(&self, now_ms: u64) -> Option<TerrainRequestBatch> {
        let available = self
            .config
            .max_in_flight_pages
            .saturating_sub(self.in_flight.len());
        let item_limit = available.min(self.config.max_batch_items);
        if item_limit == 0 {
            return None;
        }
        let mut replacement_groups = BTreeMap::<TerrainPageKey, Vec<PendingDemand>>::new();
        let mut candidates = Vec::<Vec<PendingDemand>>::new();
        for pending in self.pending.values().cloned() {
            if let Some(parent) = pending.replacement_parent {
                replacement_groups.entry(parent).or_default().push(pending);
            } else {
                candidates.push(vec![pending]);
            }
        }
        candidates.extend(replacement_groups.into_values());
        candidates.retain(|group| group.iter().all(|pending| pending.retry_at_ms <= now_ms));
        candidates.sort_by(|left, right| compare_pending_groups(left, right));
        let mut pages = Vec::new();
        let mut estimated_bytes = 0usize;
        for mut group in candidates {
            group.sort_by(|left, right| compare_demands(&left.demand, &right.demand));
            if pages.len().saturating_add(group.len()) > item_limit {
                continue;
            }
            let group_bytes = group.iter().fold(0usize, |bytes, pending| {
                bytes.saturating_add(pending.demand.estimated_encoded_bytes as usize)
            });
            if estimated_bytes.saturating_add(group_bytes) > self.config.max_batch_bytes {
                continue;
            }
            estimated_bytes = estimated_bytes.saturating_add(group_bytes);
            pages.extend(group.into_iter().map(|pending| pending.demand.identity));
        }
        (!pages.is_empty()).then_some(TerrainRequestBatch { pages })
    }

    pub fn commit_batch(&mut self, batch: &TerrainRequestBatch) -> Result<(), TerrainStreamError> {
        if batch.pages.is_empty()
            || batch.pages.iter().copied().collect::<BTreeSet<_>>().len() != batch.pages.len()
        {
            return Err(TerrainStreamError::InvalidRequestBatch);
        }
        if let Some(identity) = batch
            .pages
            .iter()
            .find(|identity| !self.pending.contains_key(identity))
        {
            return Err(TerrainStreamError::UnknownPending(*identity));
        }
        for identity in &batch.pages {
            let pending = self
                .pending
                .remove(identity)
                .ok_or(TerrainStreamError::UnknownPending(*identity))?;
            self.in_flight.insert(
                *identity,
                InFlightDemand {
                    pending,
                    obsolete: false,
                },
            );
        }
        Ok(())
    }

    pub fn next_batch(&mut self, now_ms: u64) -> Option<TerrainRequestBatch> {
        let batch = self.peek_batch(now_ms)?;
        self.commit_batch(&batch).ok()?;
        Some(batch)
    }

    pub fn complete(
        &mut self,
        identity: TerrainPageTransferIdentity,
        received_bytes: usize,
    ) -> Result<bool, TerrainStreamError> {
        let Some(entry) = self.in_flight.remove(&identity) else {
            return Err(TerrainStreamError::UnknownInFlight(identity));
        };
        if entry.obsolete {
            self.cancellation_waste_bytes = self
                .cancellation_waste_bytes
                .saturating_add(received_bytes as u64);
            Ok(false)
        } else {
            self.useful_bytes = self.useful_bytes.saturating_add(received_bytes as u64);
            Ok(true)
        }
    }

    pub fn fail(
        &mut self,
        identity: TerrainPageTransferIdentity,
        now_ms: u64,
    ) -> Result<(), TerrainStreamError> {
        let Some(mut entry) = self.in_flight.remove(&identity) else {
            return Err(TerrainStreamError::UnknownInFlight(identity));
        };
        self.failed_pages = self.failed_pages.saturating_add(1);
        if entry.obsolete {
            return Ok(());
        }
        entry.pending.attempt = entry.pending.attempt.saturating_add(1);
        let shift = u32::from(entry.pending.attempt.min(20));
        let delay = self
            .config
            .retry_base_ms
            .saturating_mul(1u64 << shift)
            .min(self.config.retry_max_ms);
        entry.pending.retry_at_ms = now_ms.saturating_add(delay);
        if self.pending.len() < self.config.max_pending_pages {
            self.pending.insert(identity, entry.pending);
        }
        Ok(())
    }

    /// Returns an unsent or preempted page to pending demand without charging retry backoff.
    ///
    /// Transport flow control is not a page failure. Keeping this separate from [`Self::fail`]
    /// prevents a full shared request window from exponentially delaying the exact-vicinity work
    /// that is ready to send as soon as a capability becomes available.
    pub fn defer(
        &mut self,
        identity: TerrainPageTransferIdentity,
    ) -> Result<(), TerrainStreamError> {
        let Some(entry) = self.in_flight.remove(&identity) else {
            return Err(TerrainStreamError::UnknownInFlight(identity));
        };
        if !entry.obsolete && self.pending.len() < self.config.max_pending_pages {
            self.pending.insert(identity, entry.pending);
        }
        Ok(())
    }

    pub fn stats(&self) -> TerrainStreamStats {
        TerrainStreamStats {
            pending_pages: self.pending.len(),
            in_flight_pages: self.in_flight.len(),
            cancelled_pending_pages: self.cancelled_pending_pages,
            obsolete_in_flight_pages: self
                .in_flight
                .values()
                .filter(|entry| entry.obsolete)
                .count(),
            useful_bytes: self.useful_bytes,
            cancellation_waste_bytes: self.cancellation_waste_bytes,
            failed_pages: self.failed_pages,
        }
    }
}

fn compare_demand_groups(left: &TerrainDemandGroup, right: &TerrainDemandGroup) -> Ordering {
    let best = |group: &TerrainDemandGroup| {
        group
            .pages
            .iter()
            .min_by(|left, right| compare_demands(left, right))
            .copied()
    };
    match (best(left), best(right)) {
        (Some(left), Some(right)) => compare_demands(&left, &right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_pending_groups(left: &[PendingDemand], right: &[PendingDemand]) -> Ordering {
    let best = |group: &[PendingDemand]| {
        group
            .iter()
            .min_by(|left, right| compare_demands(&left.demand, &right.demand))
            .map(|pending| pending.demand)
    };
    match (best(left), best(right)) {
        (Some(left), Some(right)) => compare_demands(&left, &right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_demands(left: &TerrainPageDemand, right: &TerrainPageDemand) -> Ordering {
    demand_priority(right)
        .cmp(&demand_priority(left))
        .then_with(|| left.time_to_exposure_ms.cmp(&right.time_to_exposure_ms))
        .then_with(|| left.distance_millimetres.cmp(&right.distance_millimetres))
        .then_with(|| left.identity.cmp(&right.identity))
}

fn demand_priority(demand: &TerrainPageDemand) -> u128 {
    let critical = u128::from(demand.presentation_critical) * (1u128 << 124)
        + u128::from(demand.topology_critical) * (1u128 << 120)
        + u128::from(demand.silhouette_critical) * (1u128 << 112);
    let visibility = u128::from(1_000u16.saturating_sub(demand.occlusion_confidence_millis)) + 50;
    let error = u128::from(demand.projected_error_millipixels).saturating_add(1_000);
    let deadline = u128::from(60_000u32.saturating_sub(demand.time_to_exposure_ms.min(60_000))) + 1;
    let bytes = u128::from(demand.estimated_encoded_bytes.max(1));
    critical.saturating_add(
        error
            .saturating_mul(error)
            .saturating_mul(visibility)
            .saturating_mul(deadline)
            / bytes,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainPageCacheError {
    ZeroCapacity,
    InvalidPage,
    PageTooLarge,
    PinnedCapacity,
}

impl fmt::Display for TerrainPageCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => formatter.write_str("virtual terrain cache capacity is zero"),
            Self::InvalidPage => formatter.write_str("virtual terrain cache page is invalid"),
            Self::PageTooLarge => {
                formatter.write_str("virtual terrain cache page exceeds capacity")
            }
            Self::PinnedCapacity => {
                formatter.write_str("pinned virtual terrain pages occupy the cache capacity")
            }
        }
    }
}

impl std::error::Error for TerrainPageCacheError {}

struct CachedTerrainPage {
    identity: TerrainPageTransferIdentity,
    encoded: Vec<u8>,
    last_used: u64,
    pinned: bool,
}

pub struct TerrainPageMemoryCache {
    source_identity_hash: WorldSourceIdentityHash,
    capacity_bytes: usize,
    resident_bytes: usize,
    clock: u64,
    entries: BTreeMap<[u8; 32], CachedTerrainPage>,
    by_identity: BTreeMap<TerrainPageTransferIdentity, [u8; 32]>,
}

impl TerrainPageMemoryCache {
    pub fn new(
        source_identity_hash: WorldSourceIdentityHash,
        capacity_bytes: usize,
    ) -> Result<Self, TerrainPageCacheError> {
        if capacity_bytes == 0 {
            return Err(TerrainPageCacheError::ZeroCapacity);
        }
        Ok(Self {
            source_identity_hash,
            capacity_bytes,
            resident_bytes: 0,
            clock: 0,
            entries: BTreeMap::new(),
            by_identity: BTreeMap::new(),
        })
    }

    pub fn insert(
        &mut self,
        encoded: Vec<u8>,
        pinned: bool,
    ) -> Result<TerrainPageTransferIdentity, TerrainPageCacheError> {
        if encoded.len() > self.capacity_bytes {
            return Err(TerrainPageCacheError::PageTooLarge);
        }
        let page = decode_terrain_page(&encoded, self.source_identity_hash)
            .map_err(|_| TerrainPageCacheError::InvalidPage)?;
        let identity = page_identity(&page);
        if let Some(fingerprint) = self.by_identity.get(&identity).copied()
            && let Some(existing) = self.entries.get_mut(&fingerprint)
        {
            self.clock = self.clock.wrapping_add(1);
            existing.last_used = self.clock;
            existing.pinned |= pinned;
            return Ok(identity);
        }
        let required = self.resident_bytes.saturating_add(encoded.len());
        if required > self.capacity_bytes {
            let mut victims = self
                .entries
                .iter()
                .filter(|(_, entry)| !entry.pinned)
                .map(|(fingerprint, entry)| (*fingerprint, entry.last_used, entry.encoded.len()))
                .collect::<Vec<_>>();
            victims.sort_unstable_by_key(|(fingerprint, last_used, _)| (*last_used, *fingerprint));
            let mut reclaimed = 0usize;
            let mut selected = Vec::new();
            for (fingerprint, _, bytes) in victims {
                reclaimed = reclaimed.saturating_add(bytes);
                selected.push(fingerprint);
                if required.saturating_sub(reclaimed) <= self.capacity_bytes {
                    break;
                }
            }
            if required.saturating_sub(reclaimed) > self.capacity_bytes {
                return Err(TerrainPageCacheError::PinnedCapacity);
            }
            for fingerprint in selected {
                if let Some(entry) = self.entries.remove(&fingerprint) {
                    self.by_identity.remove(&entry.identity);
                    self.resident_bytes = self.resident_bytes.saturating_sub(entry.encoded.len());
                }
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.resident_bytes = self.resident_bytes.saturating_add(encoded.len());
        self.by_identity
            .insert(identity, identity.content_fingerprint);
        self.entries.insert(
            identity.content_fingerprint,
            CachedTerrainPage {
                identity,
                encoded,
                last_used: self.clock,
                pinned,
            },
        );
        Ok(identity)
    }

    pub fn get_encoded(&mut self, identity: TerrainPageTransferIdentity) -> Option<Vec<u8>> {
        let fingerprint = *self.by_identity.get(&identity)?;
        let entry = self.entries.get_mut(&fingerprint)?;
        self.clock = self.clock.wrapping_add(1);
        entry.last_used = self.clock;
        Some(entry.encoded.clone())
    }

    pub fn contains(&self, identity: TerrainPageTransferIdentity) -> bool {
        self.by_identity.contains_key(&identity)
    }

    pub fn set_pinned(&mut self, identity: TerrainPageTransferIdentity, pinned: bool) -> bool {
        let Some(fingerprint) = self.by_identity.get(&identity).copied() else {
            return false;
        };
        let Some(entry) = self.entries.get_mut(&fingerprint) else {
            return false;
        };
        entry.pinned = pinned;
        true
    }

    pub const fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn page_identity(page: &TerrainPageV1) -> TerrainPageTransferIdentity {
    TerrainPageTransferIdentity {
        key: page.key,
        revision: page.revision,
        content_fingerprint: page.content_fingerprint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Material, TerrainPageKey, VoxelCoord, build_exact_terrain_page, encode_terrain_page,
    };

    fn source() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x44; 32])
    }

    fn page(key: TerrainPageKey, revision: u64) -> (TerrainPageTransferIdentity, Vec<u8>) {
        let page = build_exact_terrain_page(source(), key, revision, |coord: VoxelCoord| {
            if coord.y <= key.bounds().unwrap().min.y + 1 {
                Material::Stone
            } else {
                Material::Air
            }
        })
        .unwrap();
        let identity = page_identity(&page);
        (identity, encode_terrain_page(&page).unwrap())
    }

    fn demand(identity: TerrainPageTransferIdentity, error: u32) -> TerrainPageDemand {
        TerrainPageDemand {
            identity,
            projected_error_millipixels: error,
            time_to_exposure_ms: 1_000,
            distance_millimetres: 1_000,
            occlusion_confidence_millis: 0,
            presentation_critical: false,
            topology_critical: false,
            silhouette_critical: false,
            estimated_encoded_bytes: 1_024,
        }
    }

    #[test]
    fn interactive_client_pacing_supports_the_widest_atomic_replacement_batch() {
        let config = TerrainStreamConfig::INTERACTIVE_CLIENT;
        assert!(config.validates());
        let volume_parent = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        assert_eq!(
            config.max_batch_items,
            volume_parent
                .refinement_children()
                .expect("volume parent children")
                .len()
        );
        assert!(config.max_in_flight_pages >= config.max_batch_items);
    }

    #[test]
    fn interactive_client_dispatches_a_complete_volume_replacement() {
        let parent = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        let pages = parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                demand(
                    TerrainPageTransferIdentity {
                        key,
                        revision: 1,
                        content_fingerprint: [index as u8 + 1; 32],
                    },
                    100,
                )
            })
            .collect::<Vec<_>>();
        let expected = pages
            .iter()
            .map(|page| page.identity)
            .collect::<BTreeSet<_>>();
        let mut scheduler =
            TerrainStreamScheduler::new(TerrainStreamConfig::INTERACTIVE_CLIENT).unwrap();
        scheduler
            .reconcile([TerrainDemandGroup::replacement(parent, pages).unwrap()])
            .unwrap();
        let batch = scheduler.peek_batch(0).unwrap();
        assert_eq!(
            batch.pages().iter().copied().collect::<BTreeSet<_>>(),
            expected
        );
    }

    #[test]
    fn duplicate_batch_is_rejected_without_partially_committing() {
        let identity = TerrainPageTransferIdentity {
            key: TerrainPageKey::surface(0, 0, 0),
            revision: 1,
            content_fingerprint: [1; 32],
        };
        let mut scheduler =
            TerrainStreamScheduler::new(TerrainStreamConfig::INTERACTIVE_CLIENT).unwrap();
        scheduler
            .reconcile([TerrainDemandGroup::singleton(demand(identity, 100))])
            .unwrap();
        let before = scheduler.stats();
        let duplicate = TerrainRequestBatch {
            pages: vec![identity, identity],
        };

        assert_eq!(
            scheduler.commit_batch(&duplicate),
            Err(TerrainStreamError::InvalidRequestBatch)
        );
        assert_eq!(scheduler.stats(), before);
        assert_eq!(scheduler.peek_batch(0).unwrap().pages(), &[identity]);
    }

    #[test]
    fn unschedulable_atomic_replacement_fails_instead_of_remaining_pending() {
        let parent = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        let pages = parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                demand(
                    TerrainPageTransferIdentity {
                        key,
                        revision: 1,
                        content_fingerprint: [index as u8 + 1; 32],
                    },
                    100,
                )
            })
            .collect::<Vec<_>>();
        let mut scheduler = TerrainStreamScheduler::new(TerrainStreamConfig {
            max_in_flight_pages: 4,
            max_batch_items: 4,
            ..TerrainStreamConfig::INTERACTIVE_CLIENT
        })
        .unwrap();

        assert_eq!(
            scheduler.reconcile([TerrainDemandGroup::replacement(parent, pages).unwrap()]),
            Err(TerrainStreamError::UnschedulableDemandGroup)
        );
        assert_eq!(scheduler.stats().pending_pages, 0);
        assert!(scheduler.peek_batch(0).is_none());
    }

    #[test]
    fn partially_available_replacement_can_use_a_smaller_scheduler() {
        let parent = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        let pages = parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                demand(
                    TerrainPageTransferIdentity {
                        key,
                        revision: 1,
                        content_fingerprint: [index as u8 + 1; 32],
                    },
                    100,
                )
            })
            .collect::<Vec<_>>();
        let available = pages
            .iter()
            .take(4)
            .map(|page| page.identity)
            .collect::<Vec<_>>();
        let mut scheduler = TerrainStreamScheduler::new(TerrainStreamConfig {
            max_in_flight_pages: 4,
            max_batch_items: 4,
            ..TerrainStreamConfig::INTERACTIVE_CLIENT
        })
        .unwrap();

        scheduler
            .reconcile_with_available(
                [TerrainDemandGroup::replacement(parent, pages).unwrap()],
                available,
            )
            .unwrap();

        assert_eq!(scheduler.peek_batch(0).unwrap().pages.len(), 4);
    }

    #[test]
    fn replacement_estimate_must_fit_one_transport_batch() {
        let parent = TerrainPageKey::surface(1, 0, 0);
        let pages = parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                let mut page = demand(
                    TerrainPageTransferIdentity {
                        key,
                        revision: 1,
                        content_fingerprint: [index as u8 + 1; 32],
                    },
                    100,
                );
                page.estimated_encoded_bytes = TERRAIN_PAGE_MAX_COMPRESSED_BYTES as u32;
                page
            })
            .collect::<Vec<_>>();
        let mut scheduler = TerrainStreamScheduler::new(TerrainStreamConfig {
            max_batch_bytes: TERRAIN_PAGE_MAX_COMPRESSED_BYTES,
            ..TerrainStreamConfig::INTERACTIVE_CLIENT
        })
        .unwrap();

        assert_eq!(
            scheduler.reconcile([TerrainDemandGroup::replacement(parent, pages).unwrap()]),
            Err(TerrainStreamError::UnschedulableDemandGroup)
        );
        assert_eq!(scheduler.stats().pending_pages, 0);
    }

    #[test]
    fn dispatch_preserves_replacement_groups_instead_of_interleaving_pages() {
        let identity = |key: TerrainPageKey, marker: u8| TerrainPageTransferIdentity {
            key,
            revision: 1,
            content_fingerprint: [marker; 32],
        };
        let first_parent = TerrainPageKey::surface(1, 0, 0);
        let second_parent = TerrainPageKey::surface(1, 1, 0);
        let first = first_parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| demand(identity(key, index as u8 + 1), 100 - index as u32 * 30))
            .collect::<Vec<_>>();
        let second = second_parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| demand(identity(key, index as u8 + 11), 99 - index as u32))
            .collect::<Vec<_>>();
        let first_keys = first
            .iter()
            .map(|page| page.identity.key)
            .collect::<BTreeSet<_>>();
        let mut scheduler = TerrainStreamScheduler::new(TerrainStreamConfig {
            max_batch_items: 4,
            ..TerrainStreamConfig::INTERACTIVE_CLIENT
        })
        .unwrap();
        scheduler
            .reconcile([
                TerrainDemandGroup::replacement(first_parent, first).unwrap(),
                TerrainDemandGroup::replacement(second_parent, second).unwrap(),
            ])
            .unwrap();
        let batch = scheduler.next_batch(0).unwrap();
        assert_eq!(batch.pages().len(), 4);
        assert_eq!(
            batch
                .pages()
                .iter()
                .map(|identity| identity.key)
                .collect::<BTreeSet<_>>(),
            first_keys
        );
    }

    #[test]
    fn partial_cached_replacement_is_not_retransmitted() {
        let parent = TerrainPageKey::surface(1, 2, -3);
        let pages = parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                demand(
                    TerrainPageTransferIdentity {
                        key,
                        revision: 11,
                        content_fingerprint: [index as u8 + 1; 32],
                    },
                    100,
                )
            })
            .collect::<Vec<_>>();
        let first = pages[0].identity;
        let second = pages[1].identity;
        let group = TerrainDemandGroup::replacement(parent, pages.clone()).unwrap();
        let mut scheduler =
            TerrainStreamScheduler::new(TerrainStreamConfig::INTERACTIVE_CLIENT).unwrap();

        scheduler
            .reconcile_with_available([group.clone()], [first])
            .unwrap();
        let batch = scheduler.peek_batch(0).unwrap();
        assert_eq!(batch.pages().len(), 3);
        assert!(!batch.pages().contains(&first));

        scheduler
            .reconcile_with_available([group], [first, second])
            .unwrap();
        let batch = scheduler.peek_batch(0).unwrap();
        assert_eq!(batch.pages().len(), 2);
        assert!(!batch.pages().contains(&first));
        assert!(!batch.pages().contains(&second));
    }

    #[test]
    fn cache_arrival_obsoletes_the_same_identity_already_in_flight() {
        let parent = TerrainPageKey::surface(1, 2, -3);
        let pages = parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                demand(
                    TerrainPageTransferIdentity {
                        key,
                        revision: 11,
                        content_fingerprint: [index as u8 + 1; 32],
                    },
                    100,
                )
            })
            .collect::<Vec<_>>();
        let cached_while_in_flight = pages[0].identity;
        let group = TerrainDemandGroup::replacement(parent, pages).unwrap();
        let mut scheduler =
            TerrainStreamScheduler::new(TerrainStreamConfig::INTERACTIVE_CLIENT).unwrap();
        scheduler.reconcile([group.clone()]).unwrap();
        let batch = scheduler.peek_batch(0).unwrap();
        scheduler.commit_batch(&batch).unwrap();

        scheduler
            .reconcile_with_available([group], [cached_while_in_flight])
            .unwrap();
        assert_eq!(scheduler.stats().obsolete_in_flight_pages, 1);
        assert!(
            !scheduler.complete(cached_while_in_flight, 4096).unwrap(),
            "the late duplicate must not be installed or counted as useful"
        );
        assert_eq!(scheduler.stats().cancellation_waste_bytes, 4096);
        assert!(
            scheduler
                .peek_batch(0)
                .is_none_or(|batch| !batch.pages().contains(&cached_while_in_flight)),
            "a cached identity is never retransmitted"
        );
    }

    #[test]
    fn incomplete_replacement_groups_are_rejected() {
        let parent = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        let (identity, _) = page(parent.children().unwrap()[0], 1);
        assert_eq!(
            TerrainDemandGroup::replacement(parent, vec![demand(identity, 1)]),
            Err(TerrainStreamError::InvalidDemandGroup)
        );
    }

    #[test]
    fn replacement_group_partition_reports_only_complete_sibling_sets() {
        let complete_parent = TerrainPageKey::surface(2, -1, 3);
        let incomplete_parent = TerrainPageKey::surface(2, 4, -2);
        let mut requested = complete_parent
            .refinement_children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| TerrainPageTransferIdentity {
                key,
                revision: 7,
                content_fingerprint: [index as u8 + 1; 32],
            })
            .collect::<Vec<_>>();
        requested.push(TerrainPageTransferIdentity {
            key: incomplete_parent.refinement_children().unwrap()[0],
            revision: 8,
            content_fingerprint: [9; 32],
        });

        let groups = terrain_page_replacement_groups(&requested);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, complete_parent);
        assert_eq!(groups[0].1.len(), 4);
        assert!(
            groups[0]
                .1
                .iter()
                .all(|identity| identity.key.parent() == Some(complete_parent))
        );
    }

    #[test]
    fn reconcile_admits_complete_groups_without_unbounded_pending_work() {
        let parent = TerrainPageKey {
            level: 1,
            coord: [0, 0, 0],
        };
        let pages = parent
            .children()
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                let (identity, _) = page(key, 1);
                demand(identity, 10_000 - index as u32)
            })
            .collect();
        let group = TerrainDemandGroup::replacement(parent, pages).unwrap();
        let mut config = TerrainStreamConfig::DEVELOPMENT;
        config.max_pending_pages = 8;
        config.max_in_flight_pages = 8;
        config.max_batch_items = 8;
        let mut scheduler = TerrainStreamScheduler::new(config).unwrap();
        scheduler.reconcile([group]).unwrap();
        assert_eq!(scheduler.stats().pending_pages, 8);
        assert_eq!(scheduler.next_batch(0).unwrap().pages.len(), 8);
        assert_eq!(scheduler.stats().in_flight_pages, 8);
    }

    #[test]
    fn topology_and_deadline_beat_occluded_bulk_error() {
        let key = |x| TerrainPageKey {
            level: 0,
            coord: [x, 0, 0],
        };
        let (critical, _) = page(key(0), 1);
        let (bulk, _) = page(key(1), 1);
        let mut critical = demand(critical, 1);
        critical.topology_critical = true;
        critical.time_to_exposure_ms = 10;
        let mut bulk = demand(bulk, u32::MAX);
        bulk.occlusion_confidence_millis = 1_000;
        let mut config = TerrainStreamConfig::DEVELOPMENT;
        config.max_in_flight_pages = 1;
        config.max_batch_items = 1;
        let mut scheduler = TerrainStreamScheduler::new(config).unwrap();
        scheduler
            .reconcile([
                TerrainDemandGroup::singleton(bulk),
                TerrainDemandGroup::singleton(critical),
            ])
            .unwrap();
        assert_eq!(
            scheduler.next_batch(0).unwrap().pages,
            vec![critical.identity]
        );
    }

    #[test]
    fn presentation_dependencies_beat_optional_unresolved_topology() {
        let key = |x| TerrainPageKey {
            level: 0,
            coord: [x, 0, 0],
        };
        let (presentation, _) = page(key(0), 1);
        let (optional, _) = page(key(1), 1);
        let mut presentation = demand(presentation, 1);
        presentation.presentation_critical = true;
        presentation.time_to_exposure_ms = 10;
        let mut optional = demand(optional, u32::MAX);
        optional.topology_critical = true;
        optional.silhouette_critical = true;
        optional.time_to_exposure_ms = 0;
        let mut config = TerrainStreamConfig::DEVELOPMENT;
        config.max_in_flight_pages = 1;
        config.max_batch_items = 1;
        let mut scheduler = TerrainStreamScheduler::new(config).unwrap();
        scheduler
            .reconcile([
                TerrainDemandGroup::singleton(optional),
                TerrainDemandGroup::singleton(presentation),
            ])
            .unwrap();
        assert_eq!(
            scheduler.next_batch(0).unwrap().pages,
            vec![presentation.identity]
        );
    }

    #[test]
    fn obsolete_in_flight_bytes_are_accounted_as_cancellation_waste() {
        let (identity, _) = page(
            TerrainPageKey {
                level: 0,
                coord: [0, 0, 0],
            },
            1,
        );
        let mut scheduler = TerrainStreamScheduler::new(TerrainStreamConfig::DEVELOPMENT).unwrap();
        scheduler
            .reconcile([TerrainDemandGroup::singleton(demand(identity, 10))])
            .unwrap();
        scheduler.next_batch(0).unwrap();
        scheduler.reconcile([]).unwrap();
        assert!(!scheduler.complete(identity, 12_345).unwrap());
        assert_eq!(scheduler.stats().cancellation_waste_bytes, 12_345);
    }

    #[test]
    fn transport_deferral_preserves_priority_without_retry_backoff() {
        let (identity, _) = page(
            TerrainPageKey {
                level: 0,
                coord: [0, 0, 0],
            },
            1,
        );
        let mut scheduler = TerrainStreamScheduler::new(TerrainStreamConfig::DEVELOPMENT).unwrap();
        scheduler
            .reconcile([TerrainDemandGroup::singleton(demand(identity, 10))])
            .unwrap();
        assert_eq!(scheduler.next_batch(0).unwrap().pages, vec![identity]);
        scheduler.defer(identity).unwrap();
        assert_eq!(scheduler.stats().failed_pages, 0);
        assert_eq!(scheduler.next_batch(0).unwrap().pages, vec![identity]);
    }

    #[test]
    fn cache_evicts_lru_but_never_a_pinned_fallback() {
        let first = page(
            TerrainPageKey {
                level: 0,
                coord: [0, 0, 0],
            },
            1,
        );
        let second = page(
            TerrainPageKey {
                level: 0,
                coord: [1, 0, 0],
            },
            1,
        );
        let third = page(
            TerrainPageKey {
                level: 0,
                coord: [2, 0, 0],
            },
            1,
        );
        let capacity = first.1.len() + second.1.len().max(third.1.len());
        let mut cache = TerrainPageMemoryCache::new(source(), capacity).unwrap();
        cache.insert(first.1.clone(), true).unwrap();
        cache.insert(second.1.clone(), false).unwrap();
        cache.insert(third.1.clone(), false).unwrap();
        assert!(cache.get_encoded(first.0).is_some());
        assert!(cache.get_encoded(second.0).is_none());
        assert!(cache.get_encoded(third.0).is_some());
    }
}
