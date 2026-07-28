//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, and input semantics.

#[cfg(any(target_arch = "wasm32", test))]
use voxels_core::CameraState;

#[cfg(any(target_arch = "wasm32", test))]
const INTERACTION_REACH_METRES: f32 = 5.0;
#[cfg(any(target_arch = "wasm32", test))]
const INTERACTION_STREAM_MARGIN_METRES: f32 = 0.7;
#[cfg(any(target_arch = "wasm32", test))]
const CHUNK_FACE_WORDS: usize = voxels_world::CHUNK_EDGE * voxels_world::CHUNK_EDGE / 64;
#[cfg(any(target_arch = "wasm32", test))]
const CAMERA_VERTICAL_FOV_RADIANS: f32 = 68.0_f32.to_radians();
#[cfg(any(target_arch = "wasm32", test))]
const SKY_ESCAPE_MIN_FACE_CELLS: u32 =
    (voxels_world::CHUNK_EDGE * voxels_world::CHUNK_EDGE / 2) as u32;
#[cfg(target_arch = "wasm32")]
const COLLISION_READINESS_RESERVE_SECONDS: f32 = 1.0;
#[cfg(any(target_arch = "wasm32", test))]
const INVENTORY_SWIPE_THRESHOLD_CSS_PIXELS: f32 = 34.0;
#[cfg(any(target_arch = "wasm32", test))]
fn presence_heartbeat_expired(
    local_time_ms: f64,
    unanswered_ping_since_ms: f64,
    timeout_ms: u32,
) -> bool {
    local_time_ms.is_finite()
        && unanswered_ping_since_ms.is_finite()
        && local_time_ms - unanswered_ping_since_ms >= f64::from(timeout_ms)
}

#[cfg(any(target_arch = "wasm32", test))]
fn inventory_swipe(anchor: [f32; 2], current: [f32; 2]) -> Option<(i32, [f32; 2])> {
    if !anchor.into_iter().chain(current).all(f32::is_finite) {
        return None;
    }
    let delta_x = current[0] - anchor[0];
    let delta_y = current[1] - anchor[1];
    if delta_x.abs() < INVENTORY_SWIPE_THRESHOLD_CSS_PIXELS || delta_x.abs() <= delta_y.abs() * 1.15
    {
        return None;
    }
    let steps = (delta_x / INVENTORY_SWIPE_THRESHOLD_CSS_PIXELS).trunc() as i32;
    Some((
        -steps,
        [
            anchor[0] + steps as f32 * INVENTORY_SWIPE_THRESHOLD_CSS_PIXELS,
            current[1],
        ],
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn insert_chunk_aabb(
    chunks: &mut std::collections::BTreeSet<voxels_world::ChunkCoord>,
    minimum: glam::Vec3,
    maximum: glam::Vec3,
) {
    use voxels_world::{CHUNK_EDGE, ChunkCoord, VOXEL_SIZE_METRES};

    if !minimum.is_finite() || !maximum.is_finite() {
        return;
    }
    let chunk_size = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
    let minimum = (minimum / chunk_size).floor().as_ivec3();
    let maximum = (maximum / chunk_size).floor().as_ivec3();
    for z in minimum.z..=maximum.z {
        for y in minimum.y..=maximum.y {
            for x in minimum.x..=maximum.x {
                let coord = ChunkCoord::new(x, y, z);
                if coord.is_world_representable() {
                    chunks.insert(coord);
                }
            }
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn movement_stream_interest(
    eye_position: glam::Vec3,
    streaming_velocity: glam::Vec3,
    collision_lookahead_seconds: f32,
) -> Vec<voxels_world::ChunkCoord> {
    use std::collections::BTreeSet;
    use voxels_core::{PLAYER_EYE_HEIGHT_METRES, PLAYER_HEIGHT_METRES, PLAYER_RADIUS_METRES};
    use voxels_world::{CHUNK_EDGE, VOXEL_SIZE_METRES};

    let mut chunks = BTreeSet::new();
    let lookahead = if collision_lookahead_seconds.is_finite() {
        collision_lookahead_seconds.clamp(0.1, 3.0)
    } else {
        0.1
    };
    let velocity = if streaming_velocity.is_finite() {
        streaming_velocity
    } else {
        glam::Vec3::ZERO
    };
    let motion_end = eye_position + velocity * lookahead;
    let chunk_size = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
    let steps = ((motion_end - eye_position).length() / (chunk_size * 0.5))
        .ceil()
        .max(1.0) as u32;
    let horizontal_margin = PLAYER_RADIUS_METRES + VOXEL_SIZE_METRES * 2.0;
    let vertical_margin = VOXEL_SIZE_METRES * 2.0;
    for step in 0..=steps {
        let fraction = step as f32 / steps as f32;
        let eye = eye_position.lerp(motion_end, fraction);
        insert_chunk_aabb(
            &mut chunks,
            eye + glam::Vec3::new(
                -horizontal_margin,
                -PLAYER_EYE_HEIGHT_METRES - vertical_margin,
                -horizontal_margin,
            ),
            eye + glam::Vec3::new(
                horizontal_margin,
                PLAYER_HEIGHT_METRES - PLAYER_EYE_HEIGHT_METRES + vertical_margin,
                horizontal_margin,
            ),
        );
    }
    chunks.into_iter().collect()
}

/// Spectators are collisionless and read-only, so their high cruise velocity must not turn a
/// several-hundred-metre flight path into collision-critical exact-chunk traffic. Their current
/// focus still updates normally, and the full velocity continues to prioritize forward desired
/// work; walking, swimming, and gliding retain their swept collision corridor.
#[cfg(any(target_arch = "wasm32", test))]
fn exact_streaming_velocity(camera: &CameraState, streaming_velocity: glam::Vec3) -> glam::Vec3 {
    if camera.locomotion() == voxels_core::LocomotionMode::Spectator {
        glam::Vec3::ZERO
    } else {
        streaming_velocity
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn predictive_stream_position(
    position: glam::Vec3,
    velocity: glam::Vec3,
    lookahead_seconds: f32,
    maximum_lead_metres: f32,
) -> glam::Vec3 {
    let horizontal = glam::Vec2::new(velocity.x, velocity.z) * lookahead_seconds.max(0.0);
    let lead = if horizontal.length_squared() > maximum_lead_metres * maximum_lead_metres {
        horizontal.normalize_or_zero() * maximum_lead_metres
    } else {
        horizontal
    };
    position + glam::Vec3::new(lead.x, 0.0, lead.y)
}

/// Canonical chunks intersecting the current body/support, intended movement sweep, or view/edit
/// corridor. This bounded secondary interest is both scheduled and transported as collision
/// critical, keeping physics and rendering ahead of running, gliding, swimming, and edits.
#[cfg(any(target_arch = "wasm32", test))]
fn urgent_stream_interest(
    camera: &CameraState,
    streaming_velocity: glam::Vec3,
    collision_lookahead_seconds: f32,
) -> Vec<voxels_world::ChunkCoord> {
    use std::collections::BTreeSet;

    let mut chunks = movement_stream_interest(
        camera.position,
        streaming_velocity,
        collision_lookahead_seconds,
    )
    .into_iter()
    .collect::<BTreeSet<_>>();
    let view_end = camera.position
        + camera.forward() * (INTERACTION_REACH_METRES + INTERACTION_STREAM_MARGIN_METRES);
    insert_chunk_aabb(
        &mut chunks,
        camera.position.min(view_end) - glam::Vec3::splat(INTERACTION_STREAM_MARGIN_METRES),
        camera.position.max(view_end) + glam::Vec3::splat(INTERACTION_STREAM_MARGIN_METRES),
    );
    chunks.into_iter().collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(target_arch = "wasm32", test))]
struct ChunkPortalMask {
    voxel_components: Box<[u16]>,
    component_faces: Vec<u8>,
    component_face_cells: Vec<[[u64; CHUNK_FACE_WORDS]; 6]>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl ChunkPortalMask {
    fn from_chunk(chunk: &voxels_world::Chunk) -> Self {
        use std::collections::VecDeque;

        let edge = voxels_world::CHUNK_EDGE;
        let mut voxel_components = vec![0_u16; edge * edge * edge];
        let mut component_faces = vec![0_u8];
        let mut component_face_cells = vec![[[0_u64; CHUNK_FACE_WORDS]; 6]];
        let mut queue = VecDeque::new();
        for face in 0..6 {
            for face_index in 0..edge * edge {
                let [x, y, z] = Self::face_voxel(face, face_index);
                let seed = Self::voxel_index(x, y, z);
                if voxel_components[seed] != 0 || chunk.get(x, y, z).occludes_ambient() {
                    continue;
                }
                let Ok(component) = u16::try_from(component_faces.len()) else {
                    // A 32-cubed chunk cannot exhaust the label space, but keep malformed future
                    // chunk dimensions fail-closed: unlabelled air is culled rather than panicking.
                    return Self {
                        voxel_components: voxel_components.into(),
                        component_faces,
                        component_face_cells,
                    };
                };
                component_faces.push(0);
                component_face_cells.push([[0; CHUNK_FACE_WORDS]; 6]);
                voxel_components[seed] = component;
                queue.push_back([x, y, z]);
                while let Some([cell_x, cell_y, cell_z]) = queue.pop_front() {
                    let faces = &mut component_faces[usize::from(component)];
                    if cell_x == 0 {
                        *faces |= 1 << 0;
                        Self::mark_face_cell(
                            &mut component_face_cells[usize::from(component)][0],
                            cell_y + cell_z * edge,
                        );
                    }
                    if cell_x + 1 == edge {
                        *faces |= 1 << 1;
                        Self::mark_face_cell(
                            &mut component_face_cells[usize::from(component)][1],
                            cell_y + cell_z * edge,
                        );
                    }
                    if cell_y == 0 {
                        *faces |= 1 << 2;
                        Self::mark_face_cell(
                            &mut component_face_cells[usize::from(component)][2],
                            cell_x + cell_z * edge,
                        );
                    }
                    if cell_y + 1 == edge {
                        *faces |= 1 << 3;
                        Self::mark_face_cell(
                            &mut component_face_cells[usize::from(component)][3],
                            cell_x + cell_z * edge,
                        );
                    }
                    if cell_z == 0 {
                        *faces |= 1 << 4;
                        Self::mark_face_cell(
                            &mut component_face_cells[usize::from(component)][4],
                            cell_x + cell_y * edge,
                        );
                    }
                    if cell_z + 1 == edge {
                        *faces |= 1 << 5;
                        Self::mark_face_cell(
                            &mut component_face_cells[usize::from(component)][5],
                            cell_x + cell_y * edge,
                        );
                    }
                    for [next_x, next_y, next_z] in [
                        cell_x.checked_sub(1).map(|x| [x, cell_y, cell_z]),
                        (cell_x + 1 < edge).then_some([cell_x + 1, cell_y, cell_z]),
                        cell_y.checked_sub(1).map(|y| [cell_x, y, cell_z]),
                        (cell_y + 1 < edge).then_some([cell_x, cell_y + 1, cell_z]),
                        cell_z.checked_sub(1).map(|z| [cell_x, cell_y, z]),
                        (cell_z + 1 < edge).then_some([cell_x, cell_y, cell_z + 1]),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let next = Self::voxel_index(next_x, next_y, next_z);
                        if voxel_components[next] == 0
                            && !chunk.get(next_x, next_y, next_z).occludes_ambient()
                        {
                            voxel_components[next] = component;
                            queue.push_back([next_x, next_y, next_z]);
                        }
                    }
                }
            }
        }
        Self {
            voxel_components: voxel_components.into_boxed_slice(),
            component_faces,
            component_face_cells,
        }
    }

    fn mark_face_cell(words: &mut [u64; CHUNK_FACE_WORDS], index: usize) {
        words[index / 64] |= 1_u64 << (index % 64);
    }

    const fn voxel_index(x: usize, y: usize, z: usize) -> usize {
        x + y * voxels_world::CHUNK_EDGE + z * voxels_world::CHUNK_EDGE * voxels_world::CHUNK_EDGE
    }

    fn face_voxel(face: usize, index: usize) -> [usize; 3] {
        let edge = voxels_world::CHUNK_EDGE;
        let horizontal = index % edge;
        let vertical = index / edge;
        match face {
            0 => [0, horizontal, vertical],
            1 => [edge - 1, horizontal, vertical],
            2 => [horizontal, 0, vertical],
            3 => [horizontal, edge - 1, vertical],
            4 => [horizontal, vertical, 0],
            5 => [horizontal, vertical, edge - 1],
            _ => unreachable!("a chunk has six faces"),
        }
    }

    fn component_at(&self, x: usize, y: usize, z: usize) -> u16 {
        self.voxel_components[Self::voxel_index(x, y, z)]
    }

    fn component_at_face(&self, face: usize, index: usize) -> u16 {
        let [x, y, z] = Self::face_voxel(face, index);
        self.component_at(x, y, z)
    }

    fn component_opens_face(&self, component: u16, face: usize) -> bool {
        component != 0
            && self
                .component_faces
                .get(usize::from(component))
                .is_some_and(|faces| faces & (1 << face) != 0)
    }

    fn connected_neighbor_components(
        &self,
        component: u16,
        face: usize,
        neighbor: &Self,
        visible_cells: &[u64; CHUNK_FACE_WORDS],
    ) -> Vec<u16> {
        let opposite = [1, 0, 3, 2, 5, 4][face];
        let mut connected = Vec::new();
        let Some(component_cells) = self
            .component_face_cells
            .get(usize::from(component))
            .map(|faces| &faces[face])
        else {
            return connected;
        };
        for (word_index, (&component_word, &visible_word)) in
            component_cells.iter().zip(visible_cells).enumerate()
        {
            let mut candidates = component_word & visible_word;
            while candidates != 0 {
                let bit = candidates.trailing_zeros() as usize;
                let index = word_index * 64 + bit;
                let neighbor_component = neighbor.component_at_face(opposite, index);
                if neighbor.component_opens_face(neighbor_component, opposite)
                    && !connected.contains(&neighbor_component)
                {
                    connected.push(neighbor_component);
                }
                candidates &= candidates - 1;
            }
        }
        connected
    }

    fn component_visible_face_cells(
        &self,
        component: u16,
        face: usize,
        visible_cells: &[u64; CHUNK_FACE_WORDS],
    ) -> [u64; CHUNK_FACE_WORDS] {
        let mut cells = [0_u64; CHUNK_FACE_WORDS];
        if let Some(component_faces) = self.component_face_cells.get(usize::from(component)) {
            for (output, (&component, &visible)) in cells
                .iter_mut()
                .zip(component_faces[face].iter().zip(visible_cells))
            {
                *output = component & visible;
            }
        }
        cells
    }

    fn component_face_cell_count(&self, component: u16, face: usize) -> u32 {
        self.component_face_cells
            .get(usize::from(component))
            .map_or(0, |faces| {
                faces[face].iter().map(|word| word.count_ones()).sum()
            })
    }

    /// Extends portal components after solid voxels become non-occluding without rescanning the
    /// entire chunk. Removing solids can only join air regions; it cannot split one. Newly exposed
    /// interior air therefore stays unlabeled until it reaches a boundary-connected component,
    /// while a newly opened boundary propagates through the complete connected cavity at once.
    fn add_non_occluding_voxels(
        &mut self,
        chunk: &voxels_world::Chunk,
        local_voxels: &[[usize; 3]],
    ) {
        use std::collections::VecDeque;

        let edge = voxels_world::CHUNK_EDGE;
        let mut visited = vec![false; edge * edge * edge];
        let mut queue = VecDeque::new();
        for &[start_x, start_y, start_z] in local_voxels {
            let start = Self::voxel_index(start_x, start_y, start_z);
            if visited[start]
                || self.voxel_components[start] != 0
                || chunk.get(start_x, start_y, start_z).occludes_ambient()
            {
                continue;
            }
            visited[start] = true;
            queue.push_back([start_x, start_y, start_z]);
            let mut region = Vec::new();
            let mut neighboring_components = Vec::new();
            let mut touches_boundary = false;
            while let Some([x, y, z]) = queue.pop_front() {
                region.push([x, y, z]);
                touches_boundary |=
                    x == 0 || x + 1 == edge || y == 0 || y + 1 == edge || z == 0 || z + 1 == edge;
                for [next_x, next_y, next_z] in [
                    x.checked_sub(1).map(|next| [next, y, z]),
                    (x + 1 < edge).then_some([x + 1, y, z]),
                    y.checked_sub(1).map(|next| [x, next, z]),
                    (y + 1 < edge).then_some([x, y + 1, z]),
                    z.checked_sub(1).map(|next| [x, y, next]),
                    (z + 1 < edge).then_some([x, y, z + 1]),
                ]
                .into_iter()
                .flatten()
                {
                    let next = Self::voxel_index(next_x, next_y, next_z);
                    let component = self.voxel_components[next];
                    if component != 0 {
                        if !neighboring_components.contains(&component) {
                            neighboring_components.push(component);
                        }
                    } else if !visited[next]
                        && !chunk.get(next_x, next_y, next_z).occludes_ambient()
                    {
                        visited[next] = true;
                        queue.push_back([next_x, next_y, next_z]);
                    }
                }
            }
            if neighboring_components.is_empty() && !touches_boundary {
                continue;
            }
            neighboring_components.sort_unstable();
            let component = if let Some(component) = neighboring_components.first().copied() {
                component
            } else {
                let Ok(component) = u16::try_from(self.component_faces.len()) else {
                    // See `from_chunk`: leave this newly opened region unlabelled if a future
                    // chunk size ever exceeds the compact component-label representation.
                    continue;
                };
                self.component_faces.push(0);
                self.component_face_cells.push([[0; CHUNK_FACE_WORDS]; 6]);
                component
            };
            for &merged in neighboring_components.iter().skip(1) {
                for label in &mut self.voxel_components {
                    if *label == merged {
                        *label = component;
                    }
                }
                let merged_index = usize::from(merged);
                self.component_faces[usize::from(component)] |= self.component_faces[merged_index];
                self.component_faces[merged_index] = 0;
                for face in 0..6 {
                    for word in 0..CHUNK_FACE_WORDS {
                        self.component_face_cells[usize::from(component)][face][word] |=
                            self.component_face_cells[merged_index][face][word];
                        self.component_face_cells[merged_index][face][word] = 0;
                    }
                }
            }
            for [x, y, z] in region {
                self.voxel_components[Self::voxel_index(x, y, z)] = component;
                let faces = &mut self.component_faces[usize::from(component)];
                if x == 0 {
                    *faces |= 1 << 0;
                    Self::mark_face_cell(
                        &mut self.component_face_cells[usize::from(component)][0],
                        y + z * edge,
                    );
                }
                if x + 1 == edge {
                    *faces |= 1 << 1;
                    Self::mark_face_cell(
                        &mut self.component_face_cells[usize::from(component)][1],
                        y + z * edge,
                    );
                }
                if y == 0 {
                    *faces |= 1 << 2;
                    Self::mark_face_cell(
                        &mut self.component_face_cells[usize::from(component)][2],
                        x + z * edge,
                    );
                }
                if y + 1 == edge {
                    *faces |= 1 << 3;
                    Self::mark_face_cell(
                        &mut self.component_face_cells[usize::from(component)][3],
                        x + z * edge,
                    );
                }
                if z == 0 {
                    *faces |= 1 << 4;
                    Self::mark_face_cell(
                        &mut self.component_face_cells[usize::from(component)][4],
                        x + y * edge,
                    );
                }
                if z + 1 == edge {
                    *faces |= 1 << 5;
                    Self::mark_face_cell(
                        &mut self.component_face_cells[usize::from(component)][5],
                        x + y * edge,
                    );
                }
            }
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn viewport_view_cone_tangent(
    minimum_half_angle_degrees: f32,
    vertical_fov_radians: f32,
    viewport_width: u32,
    viewport_height: u32,
) -> Option<f32> {
    if !minimum_half_angle_degrees.is_finite()
        || !(0.0..90.0).contains(&minimum_half_angle_degrees)
        || !vertical_fov_radians.is_finite()
        || !(0.0..std::f32::consts::PI).contains(&vertical_fov_radians)
        || viewport_width == 0
        || viewport_height == 0
    {
        return None;
    }
    let vertical_tangent = (vertical_fov_radians * 0.5).tan();
    let horizontal_tangent = vertical_tangent * viewport_width as f32 / viewport_height as f32;
    let viewport_corner_tangent = vertical_tangent.hypot(horizontal_tangent);
    Some(viewport_corner_tangent.max(minimum_half_angle_degrees.to_radians().tan()))
}

#[cfg(any(target_arch = "wasm32", test))]
enum UpwardPortalProbe {
    Bounded,
    Pending(Vec<voxels_world::ChunkCoord>),
    Escapes(Vec<voxels_world::ChunkCoord>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(target_arch = "wasm32", test))]
struct PortalFrontier {
    source: voxels_world::ChunkCoord,
    neighbor: voxels_world::ChunkCoord,
    face: u8,
    cells: [u64; CHUNK_FACE_WORDS],
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(target_arch = "wasm32", test))]
struct ExactVolumeFrontierCap {
    chunk: voxels_world::ChunkCoord,
    face: u8,
    cells: [u64; CHUNK_FACE_WORDS],
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg(any(target_arch = "wasm32", test))]
struct EnclosedViewStreamPlan {
    chunks: Vec<voxels_world::ChunkCoord>,
    frontiers: Vec<PortalFrontier>,
}

#[cfg(any(target_arch = "wasm32", test))]
fn replace_portal_frontiers(
    current: &mut Vec<PortalFrontier>,
    replacement: &[PortalFrontier],
) -> bool {
    if current == replacement {
        return false;
    }
    current.clear();
    current.extend_from_slice(replacement);
    true
}

/// Follows only broad upward portals far enough to distinguish a bounded cavern from open sky.
///
/// This is deliberately independent of the lighting enclosure sample. A tall cavern can have an
/// unobstructed lighting probe while still requiring exact walls and a ceiling. Conversely, an
/// outdoor component must not activate a view-cone-sized exact-volume flood. Narrow shafts are
/// treated as bounded because the ordinary view-cone traversal is already tightly constrained for
/// them; the sky probe is solely an outdoor-work bound, not a visibility oracle.
#[cfg(any(target_arch = "wasm32", test))]
fn probe_upward_portals(
    origin: voxels_world::ChunkCoord,
    origin_component: u16,
    radius_chunks: i32,
    portals: &std::collections::BTreeMap<(i32, i32, i32), ChunkPortalMask>,
) -> UpwardPortalProbe {
    use std::collections::{BTreeSet, VecDeque};

    let mut visited = BTreeSet::from([(origin, origin_component)]);
    let mut queue = VecDeque::from([(origin, origin_component)]);
    let mut pending = BTreeSet::new();
    let mut probe_chunks = BTreeSet::from([origin]);
    let all_face_cells = [u64::MAX; CHUNK_FACE_WORDS];
    while let Some((current, component)) = queue.pop_front() {
        let Some(current_portals) = portals.get(&(current.x, current.y, current.z)) else {
            continue;
        };
        if current_portals.component_face_cell_count(component, 3) < SKY_ESCAPE_MIN_FACE_CELLS {
            continue;
        }
        let Some(y) = current.y.checked_add(1) else {
            continue;
        };
        let neighbor = voxels_world::ChunkCoord::new(current.x, y, current.z);
        if !neighbor.is_world_representable() {
            continue;
        }
        if y - origin.y > radius_chunks {
            return UpwardPortalProbe::Escapes(probe_chunks.into_iter().collect());
        }
        probe_chunks.insert(neighbor);
        let Some(neighbor_portals) = portals.get(&(neighbor.x, neighbor.y, neighbor.z)) else {
            pending.insert(neighbor);
            continue;
        };
        for neighbor_component in current_portals.connected_neighbor_components(
            component,
            3,
            neighbor_portals,
            &all_face_cells,
        ) {
            if visited.insert((neighbor, neighbor_component)) {
                queue.push_back((neighbor, neighbor_component));
            }
        }
    }
    if pending.is_empty() {
        UpwardPortalProbe::Bounded
    } else {
        probe_chunks.extend(pending);
        UpwardPortalProbe::Pending(probe_chunks.into_iter().collect())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn visible_portal_cells(
    camera: &CameraState,
    chunk: voxels_world::ChunkCoord,
    face: usize,
    distance_metres: f32,
    cone_tangent: f32,
) -> [u64; CHUNK_FACE_WORDS] {
    use voxels_world::{CHUNK_EDGE, VOXEL_SIZE_METRES};

    let mut visible = [0_u64; CHUNK_FACE_WORDS];
    let cell_radius = VOXEL_SIZE_METRES * 3.0_f32.sqrt() * 0.5;
    let forward = camera.forward();
    let distance_limit_squared = (distance_metres + cell_radius).powi(2);
    for face_index in 0..CHUNK_EDGE * CHUNK_EDGE {
        let [local_x, local_y, local_z] = ChunkPortalMask::face_voxel(face, face_index);
        let world_voxel = glam::IVec3::new(
            chunk.x * CHUNK_EDGE as i32 + local_x as i32,
            chunk.y * CHUNK_EDGE as i32 + local_y as i32,
            chunk.z * CHUNK_EDGE as i32 + local_z as i32,
        );
        let center = (world_voxel.as_vec3() + glam::Vec3::splat(0.5)) * VOXEL_SIZE_METRES;
        let camera_to_center = center - camera.position;
        let axial = camera_to_center.dot(forward);
        let distance_squared = camera_to_center.length_squared();
        if axial < -cell_radius || distance_squared > distance_limit_squared {
            continue;
        }
        let perpendicular_squared = (distance_squared - axial * axial).max(0.0);
        let cone_radius = axial.max(0.0) * cone_tangent + cell_radius;
        if perpendicular_squared <= cone_radius * cone_radius {
            ChunkPortalMask::mark_face_cell(&mut visible, face_index);
        }
    }
    visible
}

/// Exact-volume interest through the connected resident cave/tunnel portal graph.
///
/// A view-ray fan makes residency depend on the crosshair: a terminator or side wall disappears as
/// soon as it moves between rays even while it remains in the viewport. Instead, flood through
/// matching air cells on resident chunk faces, retaining each reachable chunk plus the immediate
/// neighbor that closes an opening. Unknown neighbors become the next streamed frontier; solid
/// neighbors remain exact occluders and stop traversal. An expanded view cone keeps the graph
/// bounded and stable beyond every screen edge, without following a shaft into outdoor air.
#[cfg(any(target_arch = "wasm32", test))]
fn enclosed_view_stream_plan(
    camera: &CameraState,
    distance_metres: f32,
    cone_tangent: f32,
    portals: &std::collections::BTreeMap<(i32, i32, i32), ChunkPortalMask>,
) -> EnclosedViewStreamPlan {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use voxels_world::{CHUNK_EDGE, VOXEL_SIZE_METRES};

    if !distance_metres.is_finite()
        || distance_metres <= 0.0
        || !cone_tangent.is_finite()
        || cone_tangent <= 0.0
    {
        return EnclosedViewStreamPlan::default();
    }
    let chunk_size = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
    let chunk_radius = chunk_size * 3.0_f32.sqrt() * 0.5;
    let forward = camera.forward();
    let origin = (camera.position / chunk_size).floor().as_ivec3();
    let origin = voxels_world::ChunkCoord::new(origin.x, origin.y, origin.z);
    let radius_chunks = (distance_metres / chunk_size).ceil().max(1.0) as i32;
    let radius_squared = i64::from(radius_chunks) * i64::from(radius_chunks);
    let directions = [
        ([-1, 0, 0], 0),
        ([1, 0, 0], 1),
        ([0, -1, 0], 2),
        ([0, 1, 0], 3),
        ([0, 0, -1], 4),
        ([0, 0, 1], 5),
    ];
    let mut chunks = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut visible_face_cells = BTreeMap::new();
    let mut frontiers = BTreeMap::<
        (voxels_world::ChunkCoord, voxels_world::ChunkCoord, u8),
        [u64; CHUNK_FACE_WORDS],
    >::new();
    let camera_voxel = (camera.position / VOXEL_SIZE_METRES).floor().as_ivec3();
    let mut origin_component = 0;
    if let Some(origin_portals) = portals.get(&(origin.x, origin.y, origin.z)) {
        let component = origin_portals.component_at(
            camera_voxel.x.rem_euclid(CHUNK_EDGE as i32) as usize,
            camera_voxel.y.rem_euclid(CHUNK_EDGE as i32) as usize,
            camera_voxel.z.rem_euclid(CHUNK_EDGE as i32) as usize,
        );
        if component != 0 {
            origin_component = component;
            visited.insert((origin, component));
            queue.push_back((origin, component));
        }
    }
    if origin_component == 0 {
        return EnclosedViewStreamPlan {
            chunks: vec![origin],
            frontiers: Vec::new(),
        };
    }
    match probe_upward_portals(origin, origin_component, radius_chunks, portals) {
        UpwardPortalProbe::Bounded => {}
        UpwardPortalProbe::Pending(chunks) => {
            // A broad upward opening may be outdoors or a tall cavern. Keep streaming the narrow
            // proof column, but do not turn classification uncertainty into an opaque Stone
            // ceiling. If the probe later proves bounded, the ordinary portal traversal below
            // publishes conservative terminators while the surface LOD remains visual authority.
            return EnclosedViewStreamPlan {
                chunks,
                frontiers: Vec::new(),
            };
        }
        UpwardPortalProbe::Escapes(chunks) => {
            // Retain the narrow probe column as streaming metadata. Dropping it would evict the
            // very portal masks that proved this component reaches sky, causing an endless
            // request/classify/evict loop outdoors. This stays O(radius), never floods the cone,
            // and contributes no frontier cover once the sky escape is known.
            return EnclosedViewStreamPlan {
                chunks,
                frontiers: Vec::new(),
            };
        }
    }
    chunks.insert(origin);
    while let Some((current, current_component)) = queue.pop_front() {
        chunks.insert(current);
        let Some(current_portals) = portals.get(&(current.x, current.y, current.z)) else {
            continue;
        };
        for ([dx, dy, dz], face) in directions {
            if !current_portals.component_opens_face(current_component, face) {
                continue;
            }
            let visible_cells = visible_face_cells
                .entry((current, face))
                .or_insert_with(|| {
                    visible_portal_cells(camera, current, face, distance_metres, cone_tangent)
                });
            let frontier_cells = current_portals.component_visible_face_cells(
                current_component,
                face,
                visible_cells,
            );
            if frontier_cells.iter().all(|word| *word == 0) {
                continue;
            }
            let (Some(x), Some(y), Some(z)) = (
                current.x.checked_add(dx),
                current.y.checked_add(dy),
                current.z.checked_add(dz),
            ) else {
                continue;
            };
            let neighbor = voxels_world::ChunkCoord::new(x, y, z);
            if !neighbor.is_world_representable() {
                continue;
            }
            let center = glam::Vec3::new(
                (x as f32 + 0.5) * chunk_size,
                (y as f32 + 0.5) * chunk_size,
                (z as f32 + 0.5) * chunk_size,
            );
            let camera_to_center = center - camera.position;
            let axial = camera_to_center.dot(forward);
            if axial < -chunk_radius {
                continue;
            }
            let perpendicular_squared =
                (camera_to_center.length_squared() - axial * axial).max(0.0);
            let cone_radius = axial.max(0.0) * cone_tangent + chunk_radius;
            if perpendicular_squared > cone_radius * cone_radius {
                continue;
            }
            let distance_squared = [
                i64::from(x) - i64::from(origin.x),
                i64::from(y) - i64::from(origin.y),
                i64::from(z) - i64::from(origin.z),
            ]
            .into_iter()
            .map(|axis| axis * axis)
            .sum::<i64>();
            if distance_squared > radius_squared {
                continue;
            }
            // Always retain the first chunk across an opening. If it is solid, it is the exact
            // wall that terminates the visible cave rather than a portal to traverse.
            chunks.insert(neighbor);
            let frontier = frontiers
                .entry((current, neighbor, face as u8))
                .or_insert([0; CHUNK_FACE_WORDS]);
            for (output, cells) in frontier.iter_mut().zip(frontier_cells) {
                *output |= cells;
            }
            let Some(neighbor_portals) = portals.get(&(x, y, z)) else {
                continue;
            };
            for neighbor_component in current_portals.connected_neighbor_components(
                current_component,
                face,
                neighbor_portals,
                visible_cells,
            ) {
                if visited.insert((neighbor, neighbor_component)) {
                    queue.push_back((neighbor, neighbor_component));
                }
            }
        }
    }
    EnclosedViewStreamPlan {
        chunks: chunks.into_iter().collect(),
        frontiers: frontiers
            .into_iter()
            .map(|((source, neighbor, face), cells)| PortalFrontier {
                source,
                neighbor,
                face,
                cells,
            })
            .collect(),
    }
}

#[cfg(test)]
fn enclosed_view_stream_interest(
    camera: &CameraState,
    distance_metres: f32,
    cone_tangent: f32,
    portals: &std::collections::BTreeMap<(i32, i32, i32), ChunkPortalMask>,
) -> Vec<voxels_world::ChunkCoord> {
    enclosed_view_stream_plan(camera, distance_metres, cone_tangent, portals).chunks
}

/// Activates an exact interest column only after every originally requested vertical chunk can
/// render. Capacity-truncated siblings must remain visible to this check so a partial column never
/// replaces the complete fallback surface.
#[cfg(any(target_arch = "wasm32", test))]
fn complete_renderable_interest_columns(
    interest: &[voxels_world::ChunkCoord],
    mut is_renderable: impl FnMut(voxels_world::ChunkCoord) -> bool,
) -> std::collections::BTreeSet<(i32, i32, i32)> {
    let mut columns =
        std::collections::BTreeMap::<(i32, i32), Vec<voxels_world::ChunkCoord>>::new();
    for &coord in interest {
        columns.entry((coord.x, coord.z)).or_default().push(coord);
    }
    let mut complete = std::collections::BTreeSet::new();
    for coords in columns.values() {
        if coords.iter().copied().all(&mut is_renderable) {
            complete.extend(
                coords
                    .iter()
                    .copied()
                    .map(|coord| (coord.x, coord.y, coord.z)),
            );
        }
    }
    complete
}

/// Activates every independently renderable chunk in an exact three-dimensional interest set.
///
/// Unlike a terrain-surface replacement, tunnel and cavern geometry does not claim an X/Z column:
/// each chunk supplements the heightfield hierarchy only at its own Y coordinate. Requiring every
/// newly discovered sibling in a column to be ready would revoke already presented tunnel walls
/// while the portal frontier streams, exposing the atmospheric background for one or more frames.
#[cfg(any(target_arch = "wasm32", test))]
fn renderable_exact_interest_chunks(
    interest: &[voxels_world::ChunkCoord],
    mut is_renderable: impl FnMut(voxels_world::ChunkCoord) -> bool,
) -> std::collections::BTreeSet<(i32, i32, i32)> {
    interest
        .iter()
        .copied()
        .filter(|coord| is_renderable(*coord))
        .map(|coord| (coord.x, coord.y, coord.z))
        .collect()
}

#[cfg(any(target_arch = "wasm32", test))]
fn exact_volume_frontier_faces(
    frontiers: &[PortalFrontier],
    mut is_renderable: impl FnMut(voxels_world::ChunkCoord) -> bool,
) -> Vec<ExactVolumeFrontierCap> {
    frontiers
        .iter()
        .filter(|frontier| is_renderable(frontier.source) && !is_renderable(frontier.neighbor))
        .map(|frontier| ExactVolumeFrontierCap {
            chunk: frontier.source,
            face: frontier.face,
            cells: frontier.cells,
        })
        .collect()
}

#[cfg(any(target_arch = "wasm32", test))]
fn camera_from_resume_values(values: [f32; 5]) -> CameraState {
    CameraState::from_persisted(
        glam::Vec3::new(values[0], values[1], values[2]),
        values[3],
        values[4],
    )
}

#[cfg(any(target_arch = "wasm32", test))]
fn virtual_terrain_column_corridor(start: [i32; 2], end: [i32; 2]) -> Vec<[i32; 2]> {
    let [mut x, mut z] = start.map(i64::from);
    let [end_x, end_z] = end.map(i64::from);
    let delta_x = (end_x - x).abs();
    let step_x = (end_x - x).signum();
    let delta_z = -(end_z - z).abs();
    let step_z = (end_z - z).signum();
    let mut error = delta_x + delta_z;
    let mut columns = Vec::new();
    loop {
        columns.push([x as i32, z as i32]);
        if x == end_x && z == end_z {
            break;
        }
        let doubled = error.saturating_mul(2);
        if doubled >= delta_z {
            error += delta_z;
            x += step_x;
        }
        if doubled <= delta_x {
            error += delta_x;
            z += step_z;
        }
    }
    columns
}

/// Keeps completed discovery products as a bounded spatial working set.
///
/// A frame-to-frame desired list is a request priority, not an ownership revocation list. Treating
/// it as the latter discarded successful lookahead work as soon as the camera crossed a 12.8 m
/// column boundary, then requested the same columns again. Current/predicted columns are mandatory;
/// the remaining cache retains the nearest useful results without unbounded growth.
#[cfg(any(target_arch = "wasm32", test))]
fn virtual_terrain_column_working_set(
    prioritized: &[[i32; 2]],
    completed: impl IntoIterator<Item = [i32; 2]>,
    capacity: usize,
) -> std::collections::BTreeSet<[i32; 2]> {
    let mut keep = prioritized
        .iter()
        .copied()
        .take(capacity)
        .collect::<std::collections::BTreeSet<_>>();
    if prioritized.is_empty() {
        return keep;
    }
    let mut remaining = completed
        .into_iter()
        .filter(|column| !keep.contains(column))
        .collect::<Vec<_>>();
    remaining.sort_by_key(|column| {
        let distance = prioritized
            .iter()
            .map(|focus| {
                let dx = i64::from(column[0]) - i64::from(focus[0]);
                let dz = i64::from(column[1]) - i64::from(focus[1]);
                dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz))
            })
            .min()
            .unwrap_or(i64::MAX);
        (distance, *column)
    });
    keep.extend(
        remaining
            .into_iter()
            .take(capacity.saturating_sub(keep.len())),
    );
    keep
}

/// Retains already published roots until a bounded spatial working set actually needs space.
///
/// A candidate cut may be temporarily incomplete while its column or directory is in flight. The
/// prior complete roots remain valid owners during that interval and must not be revoked merely
/// because they are absent from this frame's short request-priority list.
#[cfg(any(target_arch = "wasm32", test))]
fn virtual_terrain_root_working_set(
    prioritized: &[voxels_world::TerrainPageKey],
    registered: impl IntoIterator<Item = voxels_world::TerrainPageKey>,
    focus: voxels_world::TerrainPageKey,
    capacity: usize,
) -> std::collections::BTreeSet<voxels_world::TerrainPageKey> {
    let registered = registered
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut keep = prioritized
        .iter()
        .copied()
        .filter(|root| registered.contains(root))
        .take(capacity)
        .collect::<std::collections::BTreeSet<_>>();
    let mut remaining = registered
        .into_iter()
        .filter(|root| !keep.contains(root))
        .collect::<Vec<_>>();
    remaining.sort_by_key(|root| {
        let distance =
            root.coord
                .into_iter()
                .zip(focus.coord)
                .fold(0_i64, |sum, (coordinate, focus)| {
                    let delta = i64::from(coordinate) - i64::from(focus);
                    sum.saturating_add(delta.saturating_mul(delta))
                });
        (distance, *root)
    });
    keep.extend(
        remaining
            .into_iter()
            .take(capacity.saturating_sub(keep.len())),
    );
    keep
}

#[cfg(any(target_arch = "wasm32", test))]
fn virtual_terrain_edit_revision_keys(
    affected_chunks: &[voxels_world::ChunkCoord],
) -> (
    std::collections::BTreeSet<voxels_world::TerrainPageKey>,
    std::collections::BTreeSet<voxels_world::TerrainPageKey>,
) {
    let leaves = affected_chunks
        .iter()
        .map(|coord| voxels_world::TerrainPageKey::surface(0, coord.x, coord.z))
        .collect::<std::collections::BTreeSet<_>>();
    let roots = leaves
        .iter()
        .filter_map(|leaf| leaf.ancestor_at(voxels_world::TERRAIN_COVERAGE_ROOT_LEVEL))
        .collect::<std::collections::BTreeSet<_>>();
    let revision_keys = leaves
        .into_iter()
        .flat_map(|leaf| {
            (1..=voxels_world::TERRAIN_COVERAGE_ROOT_LEVEL)
                .filter_map(move |level| leaf.ancestor_at(level))
        })
        .collect();
    (roots, revision_keys)
}

#[cfg(any(target_arch = "wasm32", test))]
const CLOUD_PERIOD_METRES: f64 = 1_280_000.0;
#[cfg(any(target_arch = "wasm32", test))]
const ATMOSPHERE_MOTION_PERIOD_SECONDS: f64 = 4_096.0;

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct DerivedWorldEnvironment {
    server_time_seconds: f32,
    world_days: f64,
    day_fraction: f32,
    year_fraction: f32,
    moon_orbit_fraction: f32,
    twinkle_phase: f32,
    planet_circumference_metres: f32,
    axial_tilt_radians: f32,
    moon_orbit_inclination_radians: f32,
    celestial_seed: u64,
    celestial_revision: u64,
    weather_fraction: f32,
    weather_cycle_seconds: f32,
    cloud_offset_metres: [f32; 2],
    cloud_velocity_metres_per_second: [f32; 2],
    cloud_coverage: f32,
    cloud_base_metres: f32,
    cloud_top_metres: f32,
    weather_seed: u64,
    weather_revision: u64,
}

#[cfg(target_arch = "wasm32")]
impl DerivedWorldEnvironment {
    fn into_render_state(self) -> voxels_render::environment::WorldEnvironmentState {
        voxels_render::environment::WorldEnvironmentState {
            server_time_seconds: self.server_time_seconds,
            world_days: self.world_days,
            day_fraction: self.day_fraction,
            year_fraction: self.year_fraction,
            moon_orbit_fraction: self.moon_orbit_fraction,
            twinkle_phase: self.twinkle_phase,
            planet_circumference_metres: self.planet_circumference_metres,
            axial_tilt_radians: self.axial_tilt_radians,
            moon_orbit_inclination_radians: self.moon_orbit_inclination_radians,
            celestial_seed: self.celestial_seed,
            celestial_revision: self.celestial_revision,
            weather_fraction: self.weather_fraction,
            weather_cycle_seconds: self.weather_cycle_seconds,
            cloud_offset_metres: self.cloud_offset_metres,
            cloud_velocity_metres_per_second: self.cloud_velocity_metres_per_second,
            cloud_coverage: self.cloud_coverage,
            cloud_base_metres: self.cloud_base_metres,
            cloud_top_metres: self.cloud_top_metres,
            weather_seed: self.weather_seed,
            weather_revision: self.weather_revision,
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn world_environment_at(
    snapshot: voxels_world::protocol::WorldEnvironmentSnapshot,
    server_time_ms: f64,
) -> DerivedWorldEnvironment {
    let elapsed_seconds = (server_time_ms - snapshot.sample_server_time_ms as f64) / 1_000.0;
    let world_days = if snapshot.day_length_seconds > 0.0 {
        snapshot.world_day_number as f64
            + f64::from(snapshot.day_fraction)
            + elapsed_seconds / f64::from(snapshot.day_length_seconds)
    } else {
        snapshot.world_day_number as f64 + f64::from(snapshot.day_fraction)
    };
    let day_fraction = world_days.rem_euclid(1.0) as f32;
    let year_fraction = (world_days.rem_euclid(f64::from(snapshot.days_per_year))
        / f64::from(snapshot.days_per_year)) as f32;
    let moon_orbit_fraction = (world_days / f64::from(snapshot.moon_sidereal_orbit_days)
        + f64::from(snapshot.moon_orbit_phase_at_world_epoch))
    .rem_euclid(1.0) as f32;
    // Thirty-seven decorrelated twinkle cycles per world day remain restart-stable and freeze with
    // the authoritative celestial clock.
    let twinkle_phase = (world_days * 37.0).rem_euclid(1.0) as f32;
    let weather_fraction = if snapshot.weather_cycle_seconds > 0.0 {
        (f64::from(snapshot.weather_fraction)
            + elapsed_seconds / f64::from(snapshot.weather_cycle_seconds))
        .rem_euclid(1.0) as f32
    } else {
        snapshot.weather_fraction
    };
    let cloud_offset_metres = std::array::from_fn(|axis| {
        (f64::from(snapshot.cloud_offset_metres[axis])
            + f64::from(snapshot.cloud_velocity_metres_per_second[axis]) * elapsed_seconds)
            .rem_euclid(CLOUD_PERIOD_METRES) as f32
    });
    DerivedWorldEnvironment {
        // Absolute Unix seconds no longer have sub-frame precision when narrowed to f32. A shared
        // 4,096-second phase retains millisecond-scale precision and is an exact period of every
        // quantized precipitation lane, so wrapping cannot pop or desynchronize the lattice.
        server_time_seconds: (server_time_ms * 0.001)
            .max(0.0)
            .rem_euclid(ATMOSPHERE_MOTION_PERIOD_SECONDS) as f32,
        world_days,
        day_fraction,
        year_fraction,
        moon_orbit_fraction,
        twinkle_phase,
        planet_circumference_metres: snapshot.planet_circumference_metres,
        axial_tilt_radians: snapshot.axial_tilt_radians,
        moon_orbit_inclination_radians: snapshot.moon_orbit_inclination_radians,
        celestial_seed: snapshot.celestial_seed,
        celestial_revision: snapshot.celestial_revision,
        weather_fraction,
        weather_cycle_seconds: snapshot.weather_cycle_seconds,
        cloud_offset_metres,
        cloud_velocity_metres_per_second: snapshot.cloud_velocity_metres_per_second,
        cloud_coverage: snapshot.cloud_coverage,
        cloud_base_metres: snapshot.cloud_base_metres,
        cloud_top_metres: snapshot.cloud_top_metres,
        weather_seed: snapshot.weather_seed,
        weather_revision: snapshot.weather_revision,
    }
}

#[cfg(target_arch = "wasm32")]
mod presence_remote;
#[cfg(target_arch = "wasm32")]
pub mod remote;
#[cfg(any(target_arch = "wasm32", test))]
mod request_window;
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::presence_remote::RemotePresenceClient;
    use crate::remote::{
        RemoteChunkCompletion, RemoteEditEvent, RemoteRequestId, RemoteTerrainDirectoryCompletion,
        RemoteTerrainPageCompletion, RemoteTerrainRegionColumnCompletion, RemoteWorldClient,
        RemoteWorldError,
    };
    use crate::{
        ChunkPortalMask, predictive_stream_position, virtual_terrain_column_corridor,
        virtual_terrain_column_working_set, virtual_terrain_root_working_set, world_environment_at,
    };
    use bytemuck::{Pod, Zeroable};
    use glam::{Vec2, Vec3};
    use serde::Deserialize;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use voxels_client_config::ClientConfig;
    use voxels_core::{
        CameraState, EnclosureSample, InputState, LocomotionMode, PLAYER_EYE_HEIGHT_METRES,
        PLAYER_HEIGHT_METRES, PLAYER_RADIUS_METRES, PLAYER_SPRINT_SPEED_METRES_PER_SECOND,
        ProfileAutomation, ProfileConfig, ProfilePhase, ProfileRoute, SpectatorFlightConfig,
        VoxelHit, VoxelPhysics, probe_enclosure, raycast_voxels, voxel_segment_is_clear,
    };
    use voxels_render::environment::WorldEnvironmentState;
    use voxels_render::renderer::{
        ChunkActivationReason, HostUiAction, LocalLightVisibility, MissionControlConfig, Renderer,
        RendererConfig, RendererFeatureConfig, ScreenshotCanonicalPageState, ScreenshotCapture,
        ScreenshotFeatureState, ScreenshotMutableRenderState, ScreenshotReproductionIdentity,
        ScreenshotStreamingManifest, ScreenshotVirtualColumnState, ScreenshotVirtualRegionState,
        VirtualTerrainRenderMode, VirtualTerrainRendererError, VolumetricCloudConfig,
    };
    use voxels_render::shadow::DirectionalShadowConfig;
    use voxels_render::ui::{LiveStats, NavigationTelemetry};
    use voxels_render::virtual_terrain::{VirtualTerrainCut, VirtualTerrainView};
    use voxels_runtime::{
        AuthoritativeEditRevisions, ChunkState, CompletionStatus, DirectionalStreamPriority,
        FrameBudget, StreamConfig, StreamScheduler, revision_satisfies,
    };
    use voxels_world::protocol::{
        BrowserUserId, EDIT_CUBE_EDGE_VOXELS, EDIT_CUBE_VOLUME_VOXELS, EDIT_SPHERE_RADIUS_VOXELS,
        EDIT_SPHERE_VOLUME_VOXELS, EditAction, EditShape, EditVolume, MaterialInventory, PlayerId,
        PlayerIdentity, TerrainRegionColumn, VoxelMutation, WorldCapabilities,
        WorldEnvironmentSnapshot,
    };
    use voxels_world::{
        AtmosphereSample, BinaryMeshScratch, CHUNK_EDGE, CHUNK_VOXEL_BYTES,
        CINDER_VAULT_PORTAL_COUNT, CaveStreamInterest, Chunk, ChunkCoord, EditMap, Material,
        MeshedChunk, MeshingHalo, PortalState, SurfaceRegion, SurfaceSample,
        TERRAIN_COVERAGE_ROOT_LEVEL, TerrainDemandGroup, TerrainHierarchyNode, TerrainPageDemand,
        TerrainPageKey, TerrainPageMemoryCache, TerrainPageTransferIdentity, TerrainStreamConfig,
        TerrainStreamScheduler, VOXEL_SIZE_METRES, VoxelCoord, WorldProductPriority,
        WorldSourceIdentityHash, decode_terrain_page, encode_terrain_page,
        mesh_chunk_binary_with_scratch,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const FRAME_HISTORY_CAPACITY: usize = 512;
    const AUTOMATION_CONTRACT_VERSION: u32 = 8;
    const SNAPSHOT_SCHEMA_VERSION: u32 = 48;
    const FRAME_SAMPLE_WIDTH: u32 = 22;
    const GPU_SAMPLE_WIDTH: u32 = 15;
    const SNAPSHOT_FIELD_NAMES: &str = concat!(
        "cameraX,cameraY,cameraZ,yaw,pitch,grounded,quads,edits,",
        "residentChunks,trackedChunks,visibleChunks,drawCalls,arenaPages,arenaAllocatedMiB,arenaCapacityMiB,pendingJobs,",
        "frameMs,shadowDrawCalls,shadowCascades,loadP95Frames,loadMaxFrames,remeshP95Frames,remeshMaxFrames,waterQuads,",
        "waterDrawCalls,refractionCopyMiB,immersion,eyeDepthMetres,eyesSubmerged,swimming,targetVoxelX,targetVoxelY,",
        "targetVoxelZ,targetPresent,coreGpuMiB,cpuMs,simulationMs,streamMs,renderMs,gpuSampleId,",
        "gpuTotalMs,gpuShadowMs,gpuWorldMs,gpuWaterMs,gpuUiMs,wasmCommittedMiB,canonicalVoxelMiB,pendingMeshMiB,",
        "editLogicalMiB,totalEvictions,staleCompletions,profilePhase,profileElapsedSeconds,profileDistanceMetres,profileComplete,profileTrackedHigh,",
        "profilePendingHigh,profilePendingMeshHigh,profileArenaCapacityHighMiB,profileWasmHighMiB,profileEvictions,materialDetail,daylightPhase,surfaceRegion,",
        "cloudCoverage,screenSpaceAmbientOcclusion,gpuDepthPrepassMs,gpuAmbientOcclusionMs,ambientOcclusionMiB,depthPrepassDrawCalls,enclosure,interiorExposure,",
        "caveHeadlamp,enclosureProbeUs,localLightCandidates,activeLocalLights,clippedLocalLights,occludedLocalLights,portalRejectedLocalLights,localLightVisibilityTests,",
        "openCinderPortals,cinderPortalRevision,localLighting,placementMaterial,streamInterestRequested,streamInterestNormalized,streamInterestDesired,streamInterestTruncated,",
        "streamPlanOverflow,portalActiveChunks,portalActiveColumns,unreachablePortalActive,remoteAvatars,avatarParts,avatarDrawCalls,viewportFingerprintLow24,",
        "viewportFingerprintHigh24,terrainReady,renderCullMs,renderEncodeMs,renderSubmitMs,drawListTestedSlices,drawListSelectedSlices,surfaceWidth,",
        "surfaceHeight,devicePixelRatio,dayFraction,localSolarDayFraction,yearFraction,moonOrbitFraction,twinklePhase,latitudeDegrees,",
        "longitudeDegrees,localSiderealAngleRadians,moonIlluminatedFraction,celestialRevision,sunDirectionX,sunDirectionY,sunDirectionZ,moonDirectionX,",
        "moonDirectionY,moonDirectionZ,shadowStrength,cloudOffsetX,cloudOffsetZ,cloudVelocityX,cloudVelocityZ,weatherRevision,",
        "weatherKind,weatherFraction,precipitation,storminess,lightning,cloudDensity,cloudBaseMetres,cloudTopMetres,",
        "cloudRenderWidth,cloudRenderHeight,cloudViewSteps,cloudLightSteps,fogDensity,outdoorExposure,spectatorActive,canonicalLatticePresented,",
        "canonicalImmediateResident,canonicalImmediateRequired,terrainColumnCellsOwned,terrainColumnCellsRequired,generationQueued,generationInFlight,meshingQueued,meshingInFlight,",
        "uploadQueued,uploadInFlight,loadCompleted,loadInFlight,acceptedCompletions,collisionImmediateResident,collisionImmediateRequired,collisionLookaheadResident,",
        "collisionLookaheadRequired,collisionLookaheadSeconds,editCanonicalRequired,editCanonicalRenderable,editCanonicalOwned,enclosedViewResident,enclosedViewRequired,enclosedViewRenderable,",
        "enclosedViewOwned,virtualTerrainMode,virtualTerrainRegisteredRegions,virtualTerrainDirectoryInFlight,virtualTerrainDirectoryNodes,virtualTerrainResidentPages,virtualTerrainResidentMiB,virtualTerrainResidentPrimitives,",
        "virtualTerrainSelectedPages,virtualTerrainRequestedPages,virtualTerrainOwnerlessRoots,virtualTerrainGpuMatchesCpuCut,virtualTerrainGpuOverflowFlags,virtualTerrainGpuStackPeak,virtualTerrainGpuOwnerlessRoots,virtualTerrainStreamPending,virtualTerrainStreamInFlight,virtualTerrainCancellationWasteMiB,virtualTerrainCachePages,",
        "virtualTerrainCacheMiB,virtualTerrainColumns,virtualTerrainColumnInFlight,virtualTerrainColumnRevisionFloors,virtualTerrainCurrentColumnKnown,virtualTerrainCurrentColumnRoots,virtualTerrainCurrentColumnRegisteredRoots,virtualTerrainNearestRegisteredRootMetres,",
        "virtualTerrainColumnAccepted,virtualTerrainColumnSubmitDeferred,virtualTerrainColumnPreempted,virtualTerrainColumnTimedOut,virtualTerrainColumnOtherFailed,virtualTerrainDirectoryAccepted,virtualTerrainDirectorySubmitDeferred,virtualTerrainDirectoryPreempted,",
        "virtualTerrainDirectoryTimedOut,virtualTerrainDirectoryOtherFailed,virtualTerrainPublishedPages,virtualTerrainPublishedExactPages,virtualTerrainPublishedMinimumLevel,virtualTerrainPublishedMaximumLevel,virtualTerrainCutFingerprintLow24,virtualTerrainCutFingerprintHigh24,",
        "frameSequence,schemaVersion,",
        "sampleCount,droppedSamples",
    );
    const VIRTUAL_TERRAIN_MAX_COLUMNS: usize = 16;
    const VIRTUAL_TERRAIN_MAX_COLUMN_BATCHES_IN_FLIGHT: usize = 2;
    // Column discovery is CPU generation, not a transport-only lookup. Four columns amortize the
    // socket round trip without serializing the entire 1.5 s lookahead corridor into one stale
    // request; two such batches can use both negotiated per-client generation lanes.
    const VIRTUAL_TERRAIN_COLUMN_BATCH_SIZE: usize = 4;
    const VIRTUAL_TERRAIN_COLUMN_WORKING_SET: usize = 64;
    const VIRTUAL_TERRAIN_MAX_REGIONS: usize = 48;
    const VIRTUAL_TERRAIN_REGION_WORKING_SET: usize = 128;
    const VIRTUAL_TERRAIN_MAX_DIRECTORY_BATCHES_IN_FLIGHT: usize = 2;
    const VIRTUAL_TERRAIN_MAX_REFINEMENT_DIRECTORY_BATCHES_IN_FLIGHT: usize = 1;
    const VIRTUAL_TERRAIN_PAGE_COMPLETIONS_PER_FRAME: usize = 1;
    const VIRTUAL_TERRAIN_CACHE_UPLOADS_PER_FRAME: usize = 4;
    const VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS: u64 = 1_000;
    const VIRTUAL_TERRAIN_PAGE_CACHE_BYTES: usize = 128 * 1_024 * 1_024;
    const VIRTUAL_TERRAIN_REFINE_ABOVE_PIXELS: f64 = 0.75;
    const VIRTUAL_TERRAIN_COARSEN_BELOW_PIXELS: f64 = 0.35;
    #[derive(Clone, Copy, Debug)]
    struct EngineConfig {
        developer_controls_enabled: bool,
        fixed_step_seconds: f32,
        max_steps_per_frame: u32,
        max_edit_trackers: usize,
        stream_frame_budget: FrameBudget,
        startup_ready_radius_chunks: i32,
        stream_collision_lookahead_seconds: f32,
        stream_velocity_lookahead_seconds: f32,
        stream_view_cone_half_angle_degrees: f32,
        stream_enclosed_view_distance_metres: f32,
        view_distance_metres: f32,
        enclosure_probe_interval_ms: f64,
        enclosure_probe_distance_metres: f32,
    }

    type FrameCallback = Closure<dyn FnMut(f64)>;

    #[derive(Default)]
    struct VirtualTerrainRequestStats {
        column_accepted: u64,
        column_submit_deferred: u64,
        column_preempted: u64,
        column_timed_out: u64,
        column_other_failed: u64,
        directory_accepted: u64,
        directory_submit_deferred: u64,
        directory_preempted: u64,
        directory_timed_out: u64,
        directory_other_failed: u64,
    }

    #[derive(Default)]
    struct VirtualTerrainStreamingState {
        columns: BTreeMap<[i32; 2], TerrainRegionColumn>,
        column_in_flight: BTreeMap<[i32; 2], RemoteRequestId>,
        column_retry_after_ms: BTreeMap<[i32; 2], u64>,
        minimum_column_revisions: BTreeMap<[i32; 2], u64>,
        registered_roots: BTreeSet<TerrainPageKey>,
        registered_refinements: BTreeSet<TerrainPageKey>,
        directory_in_flight: BTreeMap<TerrainPageKey, RemoteRequestId>,
        directory_retry_after_ms: BTreeMap<TerrainPageKey, u64>,
        minimum_region_revisions: BTreeMap<TerrainPageKey, u64>,
        nodes: BTreeMap<TerrainPageKey, TerrainHierarchyNode>,
        stats: VirtualTerrainRequestStats,
    }

    fn virtual_terrain_directory_is_registered(
        state: &VirtualTerrainStreamingState,
        root: TerrainPageKey,
    ) -> bool {
        if root.is_surface() && root.level < TERRAIN_COVERAGE_ROOT_LEVEL {
            state.registered_refinements.contains(&root)
        } else {
            state.registered_roots.contains(&root)
        }
    }

    fn terrain_page_bounds_metres(key: TerrainPageKey) -> Option<([f64; 3], [f64; 3])> {
        if key.is_surface() {
            let [[minimum_x, minimum_z], [maximum_x, maximum_z]] = key.horizontal_bounds()?;
            return Some((
                [
                    f64::from(minimum_x) * 0.1,
                    f64::from(i32::MIN) * 0.1,
                    f64::from(minimum_z) * 0.1,
                ],
                [
                    f64::from(maximum_x) * 0.1,
                    f64::from(i32::MAX) * 0.1,
                    f64::from(maximum_z) * 0.1,
                ],
            ));
        }
        let bounds = key.bounds()?;
        Some((
            bounds.min.as_array().map(|value| f64::from(value) * 0.1),
            bounds.max.as_array().map(|value| f64::from(value) * 0.1),
        ))
    }

    fn terrain_page_center_metres(key: TerrainPageKey) -> [f64; 3] {
        terrain_page_bounds_metres(key).map_or([0.0; 3], |(minimum, maximum)| {
            std::array::from_fn(|axis| (minimum[axis] + maximum[axis]) * 0.5)
        })
    }

    fn terrain_page_distance_metres(key: TerrainPageKey, point: [f64; 3]) -> f64 {
        let Some((minimum, maximum)) = terrain_page_bounds_metres(key) else {
            return f64::INFINITY;
        };
        (0..3)
            .map(|axis| {
                let distance = if point[axis] < minimum[axis] {
                    minimum[axis] - point[axis]
                } else if point[axis] > maximum[axis] {
                    point[axis] - maximum[axis]
                } else {
                    0.0
                };
                distance * distance
            })
            .sum::<f64>()
            .sqrt()
    }

    fn terrain_page_demand(
        identity: TerrainPageTransferIdentity,
        node: &TerrainHierarchyNode,
        view: VirtualTerrainView,
        speed_metres_per_second: f32,
    ) -> TerrainPageDemand {
        let distance = terrain_page_distance_metres(node.key, view.camera_position_metres)
            .max(view.near_metres);
        let positional_error_millivoxels = node
            .errors
            .geometric_millivoxels
            .max(node.errors.silhouette_millivoxels)
            .max(node.errors.material_boundary_millivoxels);
        let positional_error_metres = f64::from(positional_error_millivoxels) * 0.000_1;
        let projection_scale = f64::from(view.viewport_height_pixels)
            / (2.0 * (view.vertical_fov_radians * 0.5).tan());
        let positional_pixels = positional_error_metres * projection_scale / distance;
        let normal_pixels = f64::from(node.errors.normal_milliradians)
            * 0.001
            * 0.25
            * view.wet_specular_sensitivity;
        let projected_error_millipixels =
            (positional_pixels.max(normal_pixels) * 1_000.0).clamp(0.0, f64::from(u32::MAX)) as u32;
        let speed = f64::from(speed_metres_per_second.max(0.0));
        let time_to_exposure_ms = if speed > 0.01 {
            (distance / speed * 1_000.0).clamp(0.0, 60_000.0) as u32
        } else {
            60_000
        };
        TerrainPageDemand {
            identity,
            projected_error_millipixels,
            time_to_exposure_ms,
            occlusion_confidence_millis: 0,
            topology_critical: node.errors.unresolved_topology,
            silhouette_critical: node.errors.silhouette_millivoxels > 0
                || node.errors.material_boundary_millivoxels > 0,
            estimated_encoded_bytes: node.encoded_bytes,
        }
    }

    fn resident_material(
        chunks: &BTreeMap<(i32, i32, i32), Chunk>,
        coord: VoxelCoord,
    ) -> Option<Material> {
        let chunk = chunks.get(&coord_key(coord.chunk()))?;
        let [x, y, z] = coord.local();
        Some(chunk.get(x, y, z))
    }

    fn resident_surface_sample(
        chunks: &BTreeMap<(i32, i32, i32), Chunk>,
        x: i32,
        z: i32,
        region: SurfaceRegion,
    ) -> Option<SurfaceSample> {
        let chunk_x = x.div_euclid(CHUNK_EDGE as i32);
        let chunk_z = z.div_euclid(CHUNK_EDGE as i32);
        let local_x = x.rem_euclid(CHUNK_EDGE as i32) as usize;
        let local_z = z.rem_euclid(CHUNK_EDGE as i32) as usize;
        let mut surface = None::<(i32, Material)>;
        let mut water_level = None::<i32>;
        for (&(candidate_x, _, candidate_z), chunk) in chunks {
            if candidate_x != chunk_x || candidate_z != chunk_z {
                continue;
            }
            let origin_y = chunk.coord().world_origin()[1];
            for local_y in 0..CHUNK_EDGE {
                let material = chunk.get(local_x, local_y, local_z);
                let world_y = origin_y + local_y as i32;
                if material.is_collidable() && surface.is_none_or(|(height, _)| world_y > height) {
                    surface = Some((world_y, material));
                }
                if material == Material::Water && water_level.is_none_or(|height| world_y > height)
                {
                    water_level = Some(world_y);
                }
            }
        }
        let (height, material) = surface?;
        Some(SurfaceSample {
            height,
            material,
            water_level,
            region,
            moisture: 0.5,
            temperature: 0.5,
            ridge: 0.0,
            route: None,
        })
    }

    #[derive(Clone, Copy, Default)]
    struct FrameSample {
        interval_ms: f32,
        cpu_ms: f32,
        simulation_ms: f32,
        stream_ms: f32,
        render_ms: f32,
        frame_id: u32,
        render_cull_ms: f32,
        render_encode_ms: f32,
        render_submit_ms: f32,
        tested_slices: u32,
        selected_slices: u32,
        stream_remote_ms: f32,
        stream_plan_ms: f32,
        stream_mesh_ms: f32,
        stream_publish_ms: f32,
        stream_virtual_terrain_ms: f32,
        stream_presence_ms: f32,
        stream_interest_ms: f32,
        stream_scheduler_update_ms: f32,
        stream_scheduler_admit_ms: f32,
        stream_collision_interest_ms: f32,
        stream_enclosed_interest_ms: f32,
    }

    #[derive(Clone, Copy, Default)]
    struct StreamFrameSample {
        remote_ms: f32,
        plan_ms: f32,
        mesh_ms: f32,
        publish_ms: f32,
        virtual_terrain_ms: f32,
        interest_ms: f32,
        scheduler_update_ms: f32,
        scheduler_admit_ms: f32,
        collision_interest_ms: f32,
        enclosed_interest_ms: f32,
    }

    struct FrameHistory {
        samples: [FrameSample; FRAME_HISTORY_CAPACITY],
        next: usize,
        len: usize,
        dropped: u32,
    }

    impl FrameHistory {
        fn new() -> Self {
            Self {
                samples: [FrameSample::default(); FRAME_HISTORY_CAPACITY],
                next: 0,
                len: 0,
                dropped: 0,
            }
        }

        fn push(&mut self, sample: FrameSample) {
            self.samples[self.next] = sample;
            self.next = (self.next + 1) % FRAME_HISTORY_CAPACITY;
            if self.len < FRAME_HISTORY_CAPACITY {
                self.len += 1;
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
        }

        fn drain_into(&mut self, values: &mut Vec<f32>) {
            values.push(self.len as f32);
            values.push(self.dropped as f32);
            let first = (self.next + FRAME_HISTORY_CAPACITY - self.len) % FRAME_HISTORY_CAPACITY;
            for offset in 0..self.len {
                let sample = self.samples[(first + offset) % FRAME_HISTORY_CAPACITY];
                values.extend_from_slice(&[
                    sample.interval_ms,
                    sample.cpu_ms,
                    sample.simulation_ms,
                    sample.stream_ms,
                    sample.render_ms,
                    sample.frame_id as f32,
                    sample.render_cull_ms,
                    sample.render_encode_ms,
                    sample.render_submit_ms,
                    sample.tested_slices as f32,
                    sample.selected_slices as f32,
                    sample.stream_remote_ms,
                    sample.stream_plan_ms,
                    sample.stream_mesh_ms,
                    sample.stream_publish_ms,
                    sample.stream_virtual_terrain_ms,
                    sample.stream_presence_ms,
                    sample.stream_interest_ms,
                    sample.stream_scheduler_update_ms,
                    sample.stream_scheduler_admit_ms,
                    sample.stream_collision_interest_ms,
                    sample.stream_enclosed_interest_ms,
                ]);
            }
            self.len = 0;
            self.dropped = 0;
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct CanonicalRequirement {
        coord: ChunkCoord,
        revision: u64,
    }

    #[derive(Default)]
    struct EditRequirements {
        canonical: Vec<CanonicalRequirement>,
    }

    struct EditTracker {
        target: VoxelCoord,
        started_ms: f64,
        requirements: EditRequirements,
    }

    struct PendingCanonicalMesh {
        revision: u64,
        mesh: MeshedChunk,
    }

    #[derive(Clone)]
    struct CanonicalPublication {
        server_revision: u64,
        requirements: BTreeMap<(i32, i32, i32), u64>,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct InputRecord {
        kind: u8,
        code: u8,
        buttons: u16,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        flags: u32,
    }

    const INPUT_RECORD_SIZE: usize = size_of::<InputRecord>();
    const _: () = assert!(INPUT_RECORD_SIZE == 24);
    const KIND_POINTER_MOVE: u8 = 1;
    const KIND_POINTER_DOWN: u8 = 0;
    const KIND_WHEEL: u8 = 2;
    const KIND_POINTER_UP: u8 = 3;
    const KIND_KEY_DOWN: u8 = 4;
    const KIND_KEY_UP: u8 = 5;
    const KIND_CANCEL: u8 = 6;

    fn log_gpu_error(message: &str) {
        web_sys::console::error_1(&JsValue::from_str(message));
    }

    fn reproduction_config_hash(config: &ClientConfig) -> Result<String, serde_json::Error> {
        let mut reproducible = config.clone();
        // Connection coordinates and credentials do not affect simulation, streaming policy, or
        // presentation and necessarily change in hermetic replay stacks. Every behavioral
        // transport limit and all runtime/render settings remain in the hashed value.
        reproducible.world.endpoint = "<world-endpoint>".to_owned();
        reproducible.world.presence_endpoint = "<presence-endpoint>".to_owned();
        reproducible.world.auth_subprotocol_token = "<authorization>".to_owned();
        Ok(blake3::hash(&serde_json::to_vec(&reproducible)?)
            .to_hex()
            .to_string())
    }

    fn reproduction_u64(value: &str, field: &str) -> Result<u64, String> {
        value
            .parse()
            .map_err(|_| format!("capture {field} is not an unsigned 64-bit integer"))
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionV2 {
        schema: String,
        runtime: ReproductionRuntime,
        image: ReproductionImage,
        camera: ReproductionCamera,
        world: ReproductionWorld,
        environment: ReproductionEnvironment,
        render: ReproductionRender,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionRuntime {
        build_commit: String,
        build_dirty: bool,
        build_profile: String,
        protocol_version: u16,
        client_config_hash: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionImage {
        pixel_width: u32,
        pixel_height: u32,
        device_pixel_ratio: f32,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionCamera {
        eye_metres: [f32; 3],
        velocity_metres_per_second: [f32; 3],
        yaw_radians: f32,
        pitch_radians: f32,
        vertical_fov_radians: f32,
        near_plane_metres: f32,
        far_plane_metres: f32,
        grounded: bool,
        locomotion: String,
        fluid: ReproductionFluid,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionFluid {
        immersion: f32,
        eye_depth_metres: f32,
        signed_eye_depth_metres: f32,
        surface_y_metres: f32,
        surface_known: bool,
        eyes_submerged: bool,
        swimming: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionWorld {
        world_id: String,
        source_identity_hash: String,
        seed: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionEnvironment {
        server_time_seconds: f32,
        world_days: f64,
        day_fraction: f32,
        year_fraction: f32,
        moon_orbit_fraction: f32,
        twinkle_phase: f32,
        planet_circumference_metres: f32,
        axial_tilt_radians: f32,
        moon_orbit_inclination_radians: f32,
        celestial_seed: String,
        celestial_revision: String,
        weather_fraction: f32,
        weather_cycle_seconds: f32,
        cloud_offset_metres: [f32; 2],
        cloud_velocity_metres_per_second: [f32; 2],
        cloud_coverage: f32,
        cloud_base_metres: f32,
        cloud_top_metres: f32,
        weather_seed: String,
        weather_revision: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionRender {
        world_lab_open: bool,
        features: ReproductionFeatures,
        diagnostic_sky_color: Option<[f32; 3]>,
        geometry_source_debug: bool,
        view_distance_metres: f32,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReproductionFeatures {
        shadows: bool,
        voxel_ambient_occlusion: bool,
        screen_space_ambient_occlusion: bool,
        fog: bool,
        far_terrain: bool,
        water: bool,
        target_outline: bool,
        material_detail: bool,
        cave_headlamp: bool,
        local_lighting: bool,
    }

    struct Engine {
        config: EngineConfig,
        renderer: RefCell<Renderer>,
        viewport_size: Cell<[u32; 2]>,
        camera: RefCell<CameraState>,
        reproduction_camera: Cell<Option<CameraState>>,
        reproduction_restore_camera: Cell<Option<CameraState>>,
        spectator_body: Cell<Option<CameraState>>,
        input: RefCell<InputState>,
        remote: RemoteWorldClient,
        presence: RemotePresenceClient,
        environment_snapshot: Cell<WorldEnvironmentSnapshot>,
        source_identity_hash: WorldSourceIdentityHash,
        remote_environment: (AtmosphereSample, SurfaceRegion),
        edits: RefCell<EditMap>,
        inventory: Cell<MaterialInventory>,
        edit_revisions: RefCell<AuthoritativeEditRevisions>,
        scheduler: RefCell<StreamScheduler>,
        chunks: RefCell<BTreeMap<(i32, i32, i32), Chunk>>,
        chunk_portals: RefCell<BTreeMap<(i32, i32, i32), ChunkPortalMask>>,
        chunk_halos: RefCell<BTreeMap<(i32, i32, i32), MeshingHalo>>,
        pending_meshes: RefCell<BTreeMap<(i32, i32, i32), PendingCanonicalMesh>>,
        pending_uploads: RefCell<BTreeMap<(i32, i32, i32), voxels_runtime::WorkTicket>>,
        canonical_publications: RefCell<VecDeque<CanonicalPublication>>,
        binary_mesh_scratch: RefCell<BinaryMeshScratch>,
        virtual_terrain: RefCell<VirtualTerrainStreamingState>,
        virtual_terrain_scheduler: RefCell<TerrainStreamScheduler>,
        virtual_terrain_cache: RefCell<TerrainPageMemoryCache>,
        terrain_ready: Cell<bool>,
        startup_ready: Cell<bool>,
        scope: DedicatedWorkerGlobalScope,
        callback: RefCell<Option<FrameCallback>>,
        frame_id: Cell<i32>,
        frame_sequence: Cell<u32>,
        last_time: Cell<f64>,
        simulation_accumulator: Cell<f32>,
        frame_milliseconds: Cell<f32>,
        cpu_milliseconds: Cell<f32>,
        simulation_milliseconds: Cell<f32>,
        stream_milliseconds: Cell<f32>,
        render_milliseconds: Cell<f32>,
        frame_history: RefCell<FrameHistory>,
        edit_trackers: RefCell<VecDeque<EditTracker>>,
        edit_last_ms: Cell<f32>,
        enclosure: Cell<EnclosureSample>,
        directional_light_occluded: Cell<bool>,
        last_enclosure_probe: Cell<f64>,
        enclosure_probe_microseconds: Cell<f32>,
        cinder_portal_state: Cell<PortalState>,
        cinder_portal_revision: Cell<u32>,
        cinder_stream_interest: Cell<CaveStreamInterest>,
        radial_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        portal_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        interaction_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        enclosed_view_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        enclosed_view_frontiers: RefCell<Vec<crate::PortalFrontier>>,
        surface_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        touch_inventory_drag: Cell<Option<[f32; 2]>>,
        profile: RefCell<ProfileAutomation>,
        profile_restore_camera: Cell<Option<CameraState>>,
        profile_tracked_high: Cell<usize>,
        profile_pending_high: Cell<usize>,
        profile_pending_mesh_high: Cell<usize>,
        profile_arena_capacity_high: Cell<u64>,
        profile_wasm_high: Cell<u64>,
        profile_start_evictions: Cell<u64>,
        stopped: Cell<bool>,
    }

    impl Engine {
        fn start(self: &Rc<Self>) -> Result<(), JsValue> {
            let weak = Rc::downgrade(self);
            let callback: FrameCallback = Closure::wrap(Box::new(move |time: f64| {
                if let Some(engine) = weak.upgrade() {
                    engine.frame(time);
                }
            }));
            *self.callback.borrow_mut() = Some(callback);
            self.request_frame()
        }

        fn request_frame(&self) -> Result<(), JsValue> {
            if self.stopped.get() {
                return Ok(());
            }
            let callback = self.callback.borrow();
            let callback = callback
                .as_ref()
                .ok_or_else(|| JsValue::from_str("animation callback is unavailable"))?;
            let id = self
                .scope
                .request_animation_frame(callback.as_ref().unchecked_ref())?;
            self.frame_id.set(id);
            Ok(())
        }

        fn source_identity_hash(&self) -> WorldSourceIdentityHash {
            self.remote
                .source_identity_hash()
                .unwrap_or(self.source_identity_hash)
        }

        fn screenshot_streaming_manifest(&self) -> ScreenshotStreamingManifest {
            let canonical_pages = self
                .scheduler
                .borrow()
                .statuses()
                .map(|status| ScreenshotCanonicalPageState {
                    coord: status.coord,
                    revision: status.revision,
                    phase: match status.state {
                        ChunkState::QueuedGeneration => 0,
                        ChunkState::Generating => 1,
                        ChunkState::QueuedMeshing => 2,
                        ChunkState::Meshing => 3,
                        ChunkState::QueuedUpload => 4,
                        ChunkState::Uploading => 5,
                        ChunkState::Resident => 6,
                    },
                    desired: status.desired,
                })
                .collect();
            let (virtual_columns, virtual_regions) = {
                let state = self.virtual_terrain.borrow();
                let columns = state
                    .columns
                    .keys()
                    .chain(state.column_in_flight.keys())
                    .chain(state.minimum_column_revisions.keys())
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|column| ScreenshotVirtualColumnState {
                        column,
                        resolved_revision: state
                            .columns
                            .get(&column)
                            .map(|resolved| resolved.revision),
                        minimum_revision: state
                            .minimum_column_revisions
                            .get(&column)
                            .copied()
                            .unwrap_or(0),
                        in_flight: state.column_in_flight.contains_key(&column),
                    })
                    .collect();
                let roots = state
                    .registered_roots
                    .iter()
                    .chain(state.registered_refinements.iter())
                    .chain(state.directory_in_flight.keys())
                    .chain(state.minimum_region_revisions.keys())
                    .copied()
                    .collect::<BTreeSet<_>>();
                let regions = roots
                    .into_iter()
                    .map(|root| ScreenshotVirtualRegionState {
                        root,
                        minimum_revision: state
                            .minimum_region_revisions
                            .get(&root)
                            .copied()
                            .unwrap_or(0),
                        registered: virtual_terrain_directory_is_registered(&state, root),
                        in_flight: state.directory_in_flight.contains_key(&root),
                    })
                    .collect();
                (columns, regions)
            };
            let virtual_stream = self.virtual_terrain_scheduler.borrow().stats();
            let (virtual_cache_pages, virtual_cache_bytes) = {
                let cache = self.virtual_terrain_cache.borrow();
                (cache.len(), cache.resident_bytes())
            };
            ScreenshotStreamingManifest {
                canonical_pages,
                virtual_columns,
                virtual_regions,
                virtual_pending_pages: virtual_stream.pending_pages,
                virtual_in_flight_pages: virtual_stream.in_flight_pages,
                virtual_obsolete_in_flight_pages: virtual_stream.obsolete_in_flight_pages,
                virtual_cancelled_pending_pages: virtual_stream.cancelled_pending_pages,
                virtual_useful_bytes: virtual_stream.useful_bytes,
                virtual_cancellation_waste_bytes: virtual_stream.cancellation_waste_bytes,
                virtual_failed_pages: virtual_stream.failed_pages,
                virtual_cache_pages,
                virtual_cache_bytes,
            }
        }

        fn apply_reproduction(&self, metadata: &str) -> Result<(), String> {
            let reproduction: ReproductionV2 = serde_json::from_str(metadata)
                .map_err(|error| format!("parse voxels.reproduction.v2: {error}"))?;
            if reproduction.schema != "voxels.reproduction.v2" {
                return Err(format!(
                    "unsupported reproduction schema {}",
                    reproduction.schema
                ));
            }
            if (reproduction.camera.far_plane_metres - reproduction.render.view_distance_metres)
                .abs()
                > 1.0e-3
            {
                return Err("capture camera and renderer view distances disagree".to_owned());
            }
            let identity = ScreenshotReproductionIdentity {
                build_commit: reproduction.runtime.build_commit,
                build_dirty: reproduction.runtime.build_dirty,
                build_profile: reproduction.runtime.build_profile,
                protocol_version: reproduction.runtime.protocol_version,
                client_config_hash: reproduction.runtime.client_config_hash,
            };
            let world_seed = reproduction_u64(&reproduction.world.seed, "world seed")?;
            let feature_state = ScreenshotFeatureState {
                shadows: reproduction.render.features.shadows,
                voxel_ambient_occlusion: reproduction.render.features.voxel_ambient_occlusion,
                screen_space_ambient_occlusion: reproduction
                    .render
                    .features
                    .screen_space_ambient_occlusion,
                fog: reproduction.render.features.fog,
                far_terrain: reproduction.render.features.far_terrain,
                water: reproduction.render.features.water,
                target_outline: reproduction.render.features.target_outline,
                cave_headlamp: reproduction.render.features.cave_headlamp,
                local_lighting: reproduction.render.features.local_lighting,
            };
            self.renderer
                .borrow()
                .validate_screenshot_reproduction_contract(
                    &identity,
                    &reproduction.world.world_id,
                    &reproduction.world.source_identity_hash,
                    world_seed,
                    reproduction.image.pixel_width,
                    reproduction.image.pixel_height,
                    reproduction.image.device_pixel_ratio,
                    reproduction.camera.vertical_fov_radians,
                    reproduction.camera.near_plane_metres,
                    reproduction.camera.far_plane_metres,
                    feature_state,
                )?;
            let locomotion = match reproduction.camera.locomotion.as_str() {
                "walking" => LocomotionMode::Walking,
                "gliding" => LocomotionMode::Gliding,
                "spectator" => LocomotionMode::Spectator,
                other => return Err(format!("capture has unknown locomotion mode {other}")),
            };
            let fluid = voxels_core::FluidState {
                immersion: reproduction.camera.fluid.immersion,
                eyes_submerged: reproduction.camera.fluid.eyes_submerged,
                eye_depth_metres: reproduction.camera.fluid.eye_depth_metres,
                signed_eye_depth_metres: reproduction.camera.fluid.signed_eye_depth_metres,
                surface_y_metres: reproduction.camera.fluid.surface_y_metres,
                surface_known: reproduction.camera.fluid.surface_known,
                swimming: reproduction.camera.fluid.swimming,
            };
            let camera = CameraState::for_reproduction(
                Vec3::from_array(reproduction.camera.eye_metres),
                Vec3::from_array(reproduction.camera.velocity_metres_per_second),
                reproduction.camera.yaw_radians,
                reproduction.camera.pitch_radians,
                reproduction.camera.grounded,
                locomotion,
                fluid,
            )
            .ok_or_else(|| "capture camera contains invalid values".to_owned())?;
            let environment = WorldEnvironmentState {
                server_time_seconds: reproduction.environment.server_time_seconds,
                world_days: reproduction.environment.world_days,
                day_fraction: reproduction.environment.day_fraction,
                year_fraction: reproduction.environment.year_fraction,
                moon_orbit_fraction: reproduction.environment.moon_orbit_fraction,
                twinkle_phase: reproduction.environment.twinkle_phase,
                planet_circumference_metres: reproduction.environment.planet_circumference_metres,
                axial_tilt_radians: reproduction.environment.axial_tilt_radians,
                moon_orbit_inclination_radians: reproduction
                    .environment
                    .moon_orbit_inclination_radians,
                celestial_seed: reproduction_u64(
                    &reproduction.environment.celestial_seed,
                    "celestial seed",
                )?,
                celestial_revision: reproduction_u64(
                    &reproduction.environment.celestial_revision,
                    "celestial revision",
                )?,
                weather_fraction: reproduction.environment.weather_fraction,
                weather_cycle_seconds: reproduction.environment.weather_cycle_seconds,
                cloud_offset_metres: reproduction.environment.cloud_offset_metres,
                cloud_velocity_metres_per_second: reproduction
                    .environment
                    .cloud_velocity_metres_per_second,
                cloud_coverage: reproduction.environment.cloud_coverage,
                cloud_base_metres: reproduction.environment.cloud_base_metres,
                cloud_top_metres: reproduction.environment.cloud_top_metres,
                weather_seed: reproduction_u64(
                    &reproduction.environment.weather_seed,
                    "weather seed",
                )?,
                weather_revision: reproduction_u64(
                    &reproduction.environment.weather_revision,
                    "weather revision",
                )?,
            };
            if environment != environment.sanitized() {
                return Err("capture environment contains invalid values".to_owned());
            }
            if reproduction
                .render
                .diagnostic_sky_color
                .is_some_and(|color| {
                    color
                        .into_iter()
                        .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(&channel))
                })
            {
                return Err("capture diagnostic sky color is invalid".to_owned());
            }
            let mut renderer = self.renderer.borrow_mut();
            if !renderer.set_reproduction_environment(Some(environment))
                || !renderer.set_reproduction_render_state(ScreenshotMutableRenderState {
                    world_lab_open: reproduction.render.world_lab_open,
                    diagnostic_sky_color: reproduction.render.diagnostic_sky_color,
                    geometry_source_debug: reproduction.render.geometry_source_debug,
                    material_detail: reproduction.render.features.material_detail,
                })
            {
                return Err("capture renderer state is invalid".to_owned());
            }
            drop(renderer);
            if self.reproduction_restore_camera.get().is_none() {
                self.reproduction_restore_camera
                    .set(Some(*self.camera.borrow()));
            }
            self.reproduction_camera.set(Some(camera));
            *self.camera.borrow_mut() = camera;
            self.input.borrow_mut().clear();
            self.simulation_accumulator.set(0.0);
            Ok(())
        }

        fn clear_reproduction(&self) {
            self.reproduction_camera.set(None);
            if let Some(camera) = self.reproduction_restore_camera.take() {
                *self.camera.borrow_mut() = camera;
            }
            _ = self
                .renderer
                .borrow_mut()
                .set_reproduction_environment(None);
            self.simulation_accumulator.set(0.0);
        }

        fn cached_surface_sample(&self, x: i32, z: i32) -> Result<SurfaceSample, String> {
            resident_surface_sample(&self.chunks.borrow(), x, z, self.remote_environment.1)
                .ok_or_else(|| "native surface column is not resident yet".to_owned())
        }

        fn start_profile(&self, profile_id: u32) -> bool {
            match profile_id {
                1 => self.start_stream_profile(ProfileRoute::Loop),
                2 => self.start_stream_profile(ProfileRoute::Straight),
                _ => {
                    log_gpu_error("unknown provider-neutral streaming profile");
                    false
                }
            }
        }

        fn start_stream_profile(&self, route: ProfileRoute) -> bool {
            self.input.borrow_mut().clear();
            let camera = *self.camera.borrow();
            let position = camera.position;
            self.profile_restore_camera.set(Some(camera));
            self.profile.borrow_mut().start_route_at_speed(
                position,
                route,
                (route == ProfileRoute::Straight).then_some(PLAYER_SPRINT_SPEED_METRES_PER_SECOND),
            );
            self.profile_tracked_high.set(0);
            self.profile_pending_high.set(0);
            self.profile_pending_mesh_high.set(0);
            self.profile_arena_capacity_high.set(0);
            self.profile_wasm_high.set(wasm_committed_bytes());
            self.profile_start_evictions
                .set(self.scheduler.borrow().diagnostics().total_evictions);
            true
        }

        fn frame(&self, time: f64) {
            let frame_sequence = self.frame_sequence.get().wrapping_add(1).max(1);
            self.frame_sequence.set(frame_sequence);
            let performance = self.scope.performance();
            let cpu_start = performance_now(performance.as_ref());
            self.apply_server_edits();
            let last = self.last_time.replace(time);
            let dt = if last <= 0.0 {
                1.0 / 60.0
            } else {
                ((time - last).max(0.0) / 1000.0) as f32
            };
            let frame_ms = dt * 1_000.0;
            self.frame_milliseconds
                .set(smoothed_ms(self.frame_milliseconds.get(), frame_ms));
            let simulation_start = performance_now(performance.as_ref());
            let spectator_available = self.spectator_available();
            let gliding_available = self.gliding_available();
            let mut camera = self.camera.borrow_mut();
            if self.profile.borrow().phase() == ProfilePhase::Complete
                && let Some(restore) = self.profile_restore_camera.take()
            {
                *camera = restore;
                self.input.borrow_mut().clear();
                self.simulation_accumulator.set(0.0);
            }
            camera.set_gliding_available(gliding_available);
            if !spectator_available && camera.locomotion() == LocomotionMode::Spectator {
                if let Some(body) = self.spectator_body.take() {
                    *camera = body;
                } else {
                    camera.set_locomotion(LocomotionMode::Walking);
                }
                self.input.borrow_mut().clear();
            }
            let reproducing = self.reproduction_camera.get().is_some();
            if let Some(reproduction_camera) = self.reproduction_camera.get() {
                *camera = reproduction_camera;
                self.input.borrow_mut().clear();
            }
            let profiling = !reproducing && self.profile.borrow().running();
            let chunks = self.chunks.borrow();
            let mut accumulator = if reproducing {
                0.0
            } else {
                (self.simulation_accumulator.get() + dt.min(0.1))
                    .min(self.config.fixed_step_seconds * self.config.max_steps_per_frame as f32)
            };
            if !self.startup_ready.get() {
                accumulator = 0.0;
            }
            let mut steps = 0;
            while self.startup_ready.get()
                && accumulator >= self.config.fixed_step_seconds
                && steps < self.config.max_steps_per_frame
            {
                if profiling {
                    self.profile.borrow_mut().advance_fixed_step();
                } else {
                    camera.update(
                        &self.input.borrow(),
                        self.config.fixed_step_seconds,
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            // Missing resident data is a conservative simulation boundary. Source
                            // requests are admitted by the stream scheduler, never from callbacks.
                            let material =
                                resident_material(&chunks, coord).unwrap_or(Material::Stone);
                            VoxelPhysics {
                                collidable: material.is_collidable(),
                                fluid: material.is_fluid(),
                            }
                        },
                    );
                }
                accumulator -= self.config.fixed_step_seconds;
                steps += 1;
            }
            self.simulation_accumulator.set(accumulator);
            drop(chunks);
            if profiling {
                // The provider-neutral rail bypasses gameplay integration, so carry its exact
                // motion intent explicitly through the production velocity-lookahead path. Drain
                // is stationary and therefore clears the last route velocity.
                camera.velocity = Vec3::ZERO;
            }
            if profiling && let Some(pose) = self.profile.borrow().pose() {
                let voxel_x = (pose.position_xz.x / VOXEL_SIZE_METRES).floor() as i32;
                let voxel_z = (pose.position_xz.y / VOXEL_SIZE_METRES).floor() as i32;
                match self.cached_surface_sample(voxel_x, voxel_z) {
                    Ok(surface) => {
                        let top = surface
                            .water_level
                            .unwrap_or(surface.height)
                            .max(surface.height);
                        let position = glam::Vec3::new(
                            pose.position_xz.x,
                            (top + 1) as f32 * VOXEL_SIZE_METRES
                                + voxels_core::PLAYER_EYE_HEIGHT_METRES
                                + 0.8,
                            pose.position_xz.y,
                        );
                        *camera = CameraState::spawn(position);
                        camera.velocity = Vec3::new(pose.velocity_xz.x, 0.0, pose.velocity_xz.y);
                        camera.yaw = pose.yaw;
                        camera.pitch = pose.pitch;
                    }
                    Err(error) => {
                        log_gpu_error(&format!("streaming profile surface probe failed: {error}"))
                    }
                }
            }
            if time - self.last_enclosure_probe.get() >= self.config.enclosure_probe_interval_ms {
                let probe_start = performance_now(performance.as_ref());
                let key_light_direction = self.renderer.borrow().key_light_direction();
                let chunks = self.chunks.borrow();
                let enclosure = probe_enclosure(
                    camera.position,
                    self.config.enclosure_probe_distance_metres,
                    VOXEL_SIZE_METRES,
                    |x, y, z| {
                        // Unloaded space cannot prove enclosure. Treating it as open avoids a
                        // false cave transition at residency boundaries while nearby loaded walls
                        // still darken freshly dug shafts before the camera crosses the surface.
                        resident_material(&chunks, VoxelCoord::new(x, y, z))
                            .is_some_and(Material::occludes_ambient)
                    },
                );
                let directional_light_occluded = enclosure.escaped_rays == 0
                    && raycast_voxels(
                        camera.position,
                        key_light_direction,
                        self.config.enclosure_probe_distance_metres,
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            resident_material(&chunks, VoxelCoord::new(x, y, z))
                                .is_some_and(Material::occludes_ambient)
                        },
                    )
                    .is_some();
                self.enclosure.set(enclosure);
                self.directional_light_occluded
                    .set(directional_light_occluded);
                self.last_enclosure_probe.set(time);
                self.enclosure_probe_microseconds
                    .set(((performance_now(performance.as_ref()) - probe_start) * 1_000.0) as f32);
            }
            let simulation_ms = (performance_now(performance.as_ref()) - simulation_start) as f32;
            self.simulation_milliseconds.set(smoothed_ms(
                self.simulation_milliseconds.get(),
                simulation_ms,
            ));
            let stream_start = performance_now(performance.as_ref());
            let streaming_velocity = if profiling || reproducing {
                camera.velocity
            } else {
                camera.streaming_velocity(&self.input.borrow())
            };
            let stream_breakdown =
                self.stream_world(&camera, streaming_velocity, performance.as_ref());
            let presence_start = performance_now(performance.as_ref());
            if let Some(opened) = self.remote.world_opened() {
                self.presence.ensure_session(&opened, time);
                self.environment_snapshot.set(opened.environment);
            }
            // The streaming profiler moves a synthetic camera outside gameplay simulation. It may
            // stream/render that route, but it must never update authoritative player position or
            // gain edit reach from benchmark-only motion.
            let remote_avatars =
                self.presence
                    .update(&camera, time, dt, !profiling && !reproducing);
            if let Some(error) = self.presence.take_error() {
                log_gpu_error(&format!("player presence: {error}"));
            }
            let stream_presence_ms =
                (performance_now(performance.as_ref()) - presence_start) as f32;
            let stream_ms = (performance_now(performance.as_ref()) - stream_start) as f32;
            self.stream_milliseconds
                .set(smoothed_ms(self.stream_milliseconds.get(), stream_ms));
            let target = if reproducing || camera.locomotion() == LocomotionMode::Spectator {
                None
            } else {
                let shape = self.renderer.borrow().edit_shape();
                self.dig_target(&camera, shape)
            };
            let mut renderer = self.renderer.borrow_mut();
            if renderer.screenshot_pending() {
                renderer.set_screenshot_streaming_manifest(self.screenshot_streaming_manifest());
            }
            renderer.set_spectator_available(spectator_available);
            renderer.set_spectator_active(camera.locomotion() == LocomotionMode::Spectator);
            renderer.set_remote_avatars(&remote_avatars);
            renderer.set_dig_target(target.map(|(hit, volume)| (hit.voxel, volume)));
            let server_time_ms = self.presence.estimated_server_time_ms(time);
            renderer.set_world_environment(
                world_environment_at(self.environment_snapshot.get(), server_time_ms)
                    .into_render_state(),
            );
            let (atmosphere, region) = self.remote_environment;
            renderer.set_atmosphere(atmosphere, region);
            let enclosure = self.enclosure.get();
            renderer.set_enclosure(enclosure, self.directional_light_occluded.get());
            renderer.set_route_status("NATIVE WORLD", 0);
            let stream = self.scheduler.borrow().diagnostics();
            let render = renderer.diagnostics();
            // Readiness means the frame has one complete visible terrain owner. Progressive
            // refinement may remain queued indefinitely as the error target improves; treating
            // an empty refinement queue as presentation readiness would make healthy terrain
            // appear unready while it is already covered by a valid parent cut.
            let terrain_ready = renderer.virtual_terrain_render_mode()
                == VirtualTerrainRenderMode::Visible
                && renderer
                    .virtual_terrain_cut()
                    .is_some_and(VirtualTerrainCut::is_renderable);
            self.terrain_ready.set(terrain_ready);
            let render_start = performance_now(performance.as_ref());
            let chunks = self.chunks.borrow();
            let eye_voxel = VoxelCoord::new(
                (camera.position.x / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.y / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.z / VOXEL_SIZE_METRES).floor() as i32,
            );
            let eye_chunk = eye_voxel.chunk();
            let submitted = renderer.render(
                frame_sequence,
                dt,
                &camera,
                LiveStats {
                    navigation: NavigationTelemetry {
                        eye_position_metres: camera.position.to_array(),
                        eye_voxel: eye_voxel.as_array(),
                        eye_chunk: [eye_chunk.x, eye_chunk.y, eye_chunk.z],
                        heading_degrees: camera.yaw.to_degrees().rem_euclid(360.0),
                        pitch_degrees: camera.pitch.to_degrees(),
                        speed_metres_per_second: camera.velocity.length(),
                        grounded: camera.grounded,
                        spectator: camera.locomotion() == LocomotionMode::Spectator,
                    },
                    frames_per_second: if self.frame_milliseconds.get() > 0.0 {
                        1_000.0 / self.frame_milliseconds.get()
                    } else {
                        0.0
                    },
                    frame_ms: self.frame_milliseconds.get(),
                    cpu_ms: self.cpu_milliseconds.get(),
                    gpu_ms: render.gpu_total_ms,
                    gpu_ambient_occlusion_ms: render.gpu_ambient_occlusion_ms,
                    resident_chunks: usize_to_u32(stream.resident),
                    visible_chunks: render.visible_chunks,
                    quads: render.quads,
                    water_quads: render.water_quads,
                    draw_calls: render.draw_calls,
                    water_draw_calls: render.water_draw_calls,
                    shadow_draw_calls: render.shadow_draw_calls,
                    shadow_cascades: render.shadow_cascades,
                    load_p95_frames: stream.initial_residency_latency.p95_frames,
                    load_max_frames: stream.initial_residency_latency.max_frames,
                    remesh_p95_frames: stream.remesh_latency.p95_frames,
                    remesh_max_frames: stream.remesh_latency.max_frames,
                    edit_last_ms: self.edit_last_ms.get(),
                    edit_in_flight: usize_to_u32(self.edit_trackers.borrow().len()),
                    pending_jobs: usize_to_u32(
                        stream.generation.queued + stream.meshing.queued + stream.upload.queued,
                    ),
                    core_gpu_bytes: render.core_gpu_bytes,
                    water_immersion: camera.fluid_state().immersion,
                    eye_depth_metres: camera.fluid_state().eye_depth_metres,
                    eyes_submerged: camera.fluid_state().eyes_submerged,
                    swimming: camera.fluid_state().swimming,
                    local_light_candidates: render.local_light_candidates,
                    active_local_lights: render.active_local_lights,
                    occluded_local_lights: render.occluded_local_lights,
                    portal_rejected_local_lights: render.portal_rejected_local_lights,
                    open_cinder_portals: self
                        .cinder_portal_state
                        .get()
                        .open_count(CINDER_VAULT_PORTAL_COUNT),
                    cinder_portal_revision: self.cinder_portal_revision.get(),
                    stream_interest_requested: usize_to_u32(stream.secondary_interest_requested),
                    stream_interest_desired: usize_to_u32(stream.secondary_interest_desired),
                    stream_interest_truncated: usize_to_u32(stream.secondary_interest_truncated),
                    portal_active_chunks: usize_to_u32(self.portal_active_chunks.borrow().len()),
                },
                |position, _maximum_geodesic_metres| {
                    if voxel_segment_is_clear(
                        camera.position,
                        Vec3::from_array(position),
                        VOXEL_SIZE_METRES,
                        |x, y, z| {
                            let coord = VoxelCoord::new(x, y, z);
                            let material =
                                resident_material(&chunks, coord).unwrap_or(Material::Stone);
                            material.occludes_ambient() && material.emission().is_none()
                        },
                    ) {
                        LocalLightVisibility::Visible
                    } else {
                        LocalLightVisibility::Occluded
                    }
                },
                || performance_now(performance.as_ref()),
            );
            if submitted
                && self
                    .scheduler
                    .borrow()
                    .vicinity_readiness(self.config.startup_ready_radius_chunks)
                    .is_ready()
            {
                self.startup_ready.set(true);
            }
            drop(chunks);
            let rendered = renderer.diagnostics();
            drop(renderer);
            self.update_edit_convergence(time, submitted);
            if self.profile.borrow().phase() != ProfilePhase::Idle {
                let pending = stream.generation.queued
                    + stream.generation.in_flight
                    + stream.meshing.queued
                    + stream.meshing.in_flight
                    + stream.upload.queued
                    + stream.upload.in_flight;
                self.profile_tracked_high
                    .set(self.profile_tracked_high.get().max(stream.tracked));
                self.profile_pending_high
                    .set(self.profile_pending_high.get().max(pending));
                self.profile_pending_mesh_high.set(
                    self.profile_pending_mesh_high
                        .get()
                        .max(self.pending_meshes.borrow().len()),
                );
                self.profile_arena_capacity_high.set(
                    self.profile_arena_capacity_high
                        .get()
                        .max(rendered.arena_capacity_bytes),
                );
                self.profile_wasm_high
                    .set(self.profile_wasm_high.get().max(wasm_committed_bytes()));
                if self.profile.borrow().phase() == ProfilePhase::Drain
                    && terrain_ready
                    && submitted
                {
                    self.profile.borrow_mut().complete_drain();
                }
            }
            let render_ms = (performance_now(performance.as_ref()) - render_start) as f32;
            self.render_milliseconds
                .set(smoothed_ms(self.render_milliseconds.get(), render_ms));
            let cpu_ms = (performance_now(performance.as_ref()) - cpu_start) as f32;
            self.cpu_milliseconds
                .set(smoothed_ms(self.cpu_milliseconds.get(), cpu_ms));
            self.frame_history.borrow_mut().push(FrameSample {
                interval_ms: frame_ms,
                cpu_ms,
                simulation_ms,
                stream_ms,
                render_ms,
                frame_id: frame_sequence,
                render_cull_ms: rendered.cpu_cull_ms,
                render_encode_ms: rendered.cpu_encode_ms,
                render_submit_ms: rendered.cpu_submit_ms,
                tested_slices: rendered.draw_list_tested_slices,
                selected_slices: rendered.draw_list_selected_slices,
                stream_remote_ms: stream_breakdown.remote_ms,
                stream_plan_ms: stream_breakdown.plan_ms,
                stream_mesh_ms: stream_breakdown.mesh_ms,
                stream_publish_ms: stream_breakdown.publish_ms,
                stream_virtual_terrain_ms: stream_breakdown.virtual_terrain_ms,
                stream_presence_ms,
                stream_interest_ms: stream_breakdown.interest_ms,
                stream_scheduler_update_ms: stream_breakdown.scheduler_update_ms,
                stream_scheduler_admit_ms: stream_breakdown.scheduler_admit_ms,
                stream_collision_interest_ms: stream_breakdown.collision_interest_ms,
                stream_enclosed_interest_ms: stream_breakdown.enclosed_interest_ms,
            });
            if let Err(error) = self.request_frame() {
                web_sys::console::error_1(&error);
                self.stopped.set(true);
            }
        }

        fn stream_world(
            &self,
            camera: &CameraState,
            streaming_velocity: Vec3,
            performance: Option<&web_sys::Performance>,
        ) -> StreamFrameSample {
            let remote_start = performance_now(performance);
            self.drain_remote_generation();
            let remote_ms = (performance_now(performance) - remote_start) as f32;
            let plan_start = performance_now(performance);
            // Lead the desired cylinder rather than only sorting it toward a prediction outside
            // the window. The current camera remains at least one chunk inside the trailing edge,
            // while newly exposed forward chunks receive real network and generation lead time.
            let exact_load_radius = self.scheduler.borrow().config().load_radius_chunks;
            let exact_lead_metres =
                (exact_load_radius - 1).max(0) as f32 * CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
            let focus = world_to_chunk(predictive_stream_position(
                camera.position,
                streaming_velocity,
                self.config.stream_velocity_lookahead_seconds,
                exact_lead_metres,
            ));
            let exact_streaming_velocity =
                crate::exact_streaming_velocity(camera, streaming_velocity);
            let collision_interest_start = performance_now(performance);
            let collision_interest = if camera.locomotion() == LocomotionMode::Spectator {
                Vec::new()
            } else {
                self.collision_stream_interest(
                    camera,
                    exact_streaming_velocity,
                    self.config.stream_collision_lookahead_seconds,
                )
            };
            let collision_interest_ms =
                (performance_now(performance) - collision_interest_start) as f32;
            let enclosed_interest_start = performance_now(performance);
            let enclosed_view_plan = self.enclosed_view_stream_plan(camera);
            let enclosed_view_interest = &enclosed_view_plan.chunks;
            let enclosed_view_frontiers_changed = crate::replace_portal_frontiers(
                &mut self.enclosed_view_frontiers.borrow_mut(),
                &enclosed_view_plan.frontiers,
            );
            let enclosed_interest_ms =
                (performance_now(performance) - enclosed_interest_start) as f32;
            // Canonical chunks are simulation and collision data only. Visible terrain demand is
            // owned entirely by the virtual page scheduler, so no camera-driven visual chunk
            // corridor or secondary terrain-height hint participates in this scheduler.
            let mut urgent_interest = collision_interest.clone();
            urgent_interest.extend(enclosed_view_interest.iter().copied());
            let interest = urgent_interest.clone();
            let priority_hint = directional_stream_priority(
                camera,
                streaming_velocity,
                CHUNK_EDGE as f32 * VOXEL_SIZE_METRES,
                self.config.stream_velocity_lookahead_seconds,
                self.config.stream_view_cone_half_angle_degrees,
            );
            let interest_ms = (performance_now(performance) - plan_start) as f32;
            let (focus_changed, scheduler_update_ms, work, scheduler_admit_ms) = {
                let mut scheduler = self.scheduler.borrow_mut();
                let update_start = performance_now(performance);
                let changed = scheduler.update_focus_with_interest(focus, &interest);
                let scheduler_update_ms = (performance_now(performance) - update_start) as f32;
                let admit_start = performance_now(performance);
                let work = scheduler.schedule_frame_prioritized_with_urgency(
                    self.config.stream_frame_budget,
                    priority_hint,
                    &urgent_interest,
                );
                (
                    changed,
                    scheduler_update_ms,
                    work,
                    (performance_now(performance) - admit_start) as f32,
                )
            };
            if focus_changed {
                let scheduler = self.scheduler.borrow();
                self.remote.cancel_chunk_batches_outside(|coord| {
                    scheduler.status(coord).is_some_and(|status| status.desired)
                });
            }
            let mut uploaded = false;

            let urgent_interest_keys: BTreeSet<_> =
                urgent_interest.iter().copied().map(coord_key).collect();
            let mut collision_generation = Vec::new();
            let mut background_generation = Vec::new();
            for ticket in work.generation {
                let dx = i64::from(ticket.coord.x) - i64::from(focus.x);
                let dz = i64::from(ticket.coord.z) - i64::from(focus.z);
                let radius = i64::from(self.config.startup_ready_radius_chunks);
                if urgent_interest_keys.contains(&coord_key(ticket.coord))
                    || (!self.startup_ready.get() && dx * dx + dz * dz <= radius * radius)
                {
                    collision_generation.push(ticket);
                } else {
                    background_generation.push(ticket);
                }
            }
            self.submit_generation_batch(
                WorldProductPriority::CollisionCritical,
                collision_generation,
            );
            self.submit_generation_batch(WorldProductPriority::VisibleChunk, background_generation);
            let plan_ms = (performance_now(performance) - plan_start) as f32;
            let mesh_start = performance_now(performance);
            {
                let chunks = self.chunks.borrow();
                let halos = self.chunk_halos.borrow();
                for ticket in work.meshing {
                    let Some(chunk) = chunks.get(&coord_key(ticket.coord)) else {
                        continue;
                    };
                    let Some(halo) = halos.get(&coord_key(ticket.coord)) else {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                        continue;
                    };
                    let mut halo_contract_valid = true;
                    let mesh = mesh_chunk_binary_with_scratch(
                        chunk,
                        |x, y, z| {
                            let Some(material) = halo.sample_world(x, y, z) else {
                                halo_contract_valid = false;
                                return Material::Stone;
                            };
                            material
                        },
                        &mut self.binary_mesh_scratch.borrow_mut(),
                    );
                    if !halo_contract_valid {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                        web_sys::console::error_1(&JsValue::from_str(
                            "world source meshing halo omitted a required shell coordinate",
                        ));
                        continue;
                    }
                    self.pending_meshes.borrow_mut().insert(
                        coord_key(ticket.coord),
                        PendingCanonicalMesh {
                            revision: ticket.revision,
                            mesh,
                        },
                    );
                    let _ = self.scheduler.borrow_mut().complete(ticket);
                }
            }
            let mesh_ms = (performance_now(performance) - mesh_start) as f32;
            let publish_start = performance_now(performance);
            for ticket in work.upload {
                self.pending_uploads
                    .borrow_mut()
                    .insert(coord_key(ticket.coord), ticket);
            }
            uploaded |= self.publish_ready_canonical_cuts();
            let evictions = self.scheduler.borrow_mut().drain_evictions();
            let evicted = !evictions.is_empty();
            if !evictions.is_empty() {
                let mut chunks = self.chunks.borrow_mut();
                let mut portals = self.chunk_portals.borrow_mut();
                let mut halos = self.chunk_halos.borrow_mut();
                let mut pending = self.pending_meshes.borrow_mut();
                let mut pending_uploads = self.pending_uploads.borrow_mut();
                let mut renderer = self.renderer.borrow_mut();
                for eviction in evictions {
                    let key = coord_key(eviction.coord);
                    chunks.remove(&key);
                    portals.remove(&key);
                    halos.remove(&key);
                    pending.remove(&key);
                    pending_uploads.remove(&key);
                    renderer.remove_chunk(eviction.coord);
                }
            }
            if focus_changed || uploaded || evicted || enclosed_view_frontiers_changed {
                self.reconcile_chunk_activation(
                    focus,
                    &collision_interest,
                    enclosed_view_interest,
                    &enclosed_view_plan.frontiers,
                    &[],
                );
            }
            let publish_ms = (performance_now(performance) - publish_start) as f32;
            let virtual_terrain_start = performance_now(performance);
            self.stream_virtual_terrain(camera, streaming_velocity);
            let virtual_terrain_ms = (performance_now(performance) - virtual_terrain_start) as f32;
            StreamFrameSample {
                remote_ms,
                plan_ms,
                mesh_ms,
                publish_ms,
                virtual_terrain_ms,
                interest_ms,
                scheduler_update_ms,
                scheduler_admit_ms,
                collision_interest_ms,
                enclosed_interest_ms,
            }
        }

        fn register_canonical_publication(
            &self,
            server_revision: u64,
            requirements: &[CanonicalRequirement],
        ) {
            if requirements.is_empty() {
                return;
            }
            let replacement = requirements
                .iter()
                .map(|requirement| (coord_key(requirement.coord), requirement.revision))
                .collect::<BTreeMap<_, _>>();
            let replacement_keys = replacement.keys().copied().collect::<BTreeSet<_>>();
            let mut merged = BTreeMap::new();
            let mut publications = self.canonical_publications.borrow_mut();
            publications.retain(|publication| {
                let overlaps = publication
                    .requirements
                    .keys()
                    .any(|key| replacement_keys.contains(key));
                if overlaps {
                    merged.extend(
                        publication
                            .requirements
                            .iter()
                            .map(|(key, revision)| (*key, *revision)),
                    );
                }
                !overlaps
            });
            // The newest scheduler capability wins for overlapping chunks. Non-overlapping chunks
            // from an unfinished older edit remain in the same atomic cut.
            merged.extend(replacement);
            publications.push_back(CanonicalPublication {
                server_revision,
                requirements: merged,
            });
        }

        fn publish_ready_canonical_cuts(&self) -> bool {
            let publications = self
                .canonical_publications
                .borrow()
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let mut uploaded_any = false;

            for publication in publications {
                let current = {
                    let scheduler = self.scheduler.borrow();
                    publication
                        .requirements
                        .iter()
                        .filter_map(|(key, revision)| {
                            let coord = ChunkCoord::new(key.0, key.1, key.2);
                            scheduler
                                .status(coord)
                                .is_some_and(|status| status.revision == *revision)
                                .then_some((*key, *revision))
                        })
                        .collect::<BTreeMap<_, _>>()
                };
                if current.is_empty() {
                    self.canonical_publications
                        .borrow_mut()
                        .retain(|candidate| {
                            candidate.server_revision != publication.server_revision
                        });
                    continue;
                }
                if current != publication.requirements
                    && let Some(candidate) = self
                        .canonical_publications
                        .borrow_mut()
                        .iter_mut()
                        .find(|candidate| candidate.server_revision == publication.server_revision)
                {
                    candidate.requirements.clone_from(&current);
                }

                let ready = {
                    let meshes = self.pending_meshes.borrow();
                    let uploads = self.pending_uploads.borrow();
                    current.iter().all(|(key, revision)| {
                        meshes
                            .get(key)
                            .is_some_and(|mesh| mesh.revision == *revision)
                            && uploads
                                .get(key)
                                .is_some_and(|ticket| ticket.revision == *revision)
                    })
                };
                if !ready {
                    continue;
                }

                let uploaded = {
                    let chunks = self.chunks.borrow();
                    let meshes = self.pending_meshes.borrow();
                    let mut cut = Vec::with_capacity(current.len());
                    for key in current.keys() {
                        let Some(chunk) = chunks.get(key) else {
                            cut.clear();
                            break;
                        };
                        let Some(mesh) = meshes.get(key) else {
                            cut.clear();
                            break;
                        };
                        cut.push((chunk, &mesh.mesh));
                    }
                    cut.len() == current.len()
                        && self.renderer.borrow_mut().upload_chunks_atomic(cut)
                };
                if uploaded {
                    let mut uploads = self.pending_uploads.borrow_mut();
                    let mut meshes = self.pending_meshes.borrow_mut();
                    let mut scheduler = self.scheduler.borrow_mut();
                    for key in current.keys() {
                        if let Some(ticket) = uploads.remove(key) {
                            let _ = scheduler.complete(ticket);
                        }
                        meshes.remove(key);
                    }
                    self.canonical_publications
                        .borrow_mut()
                        .retain(|candidate| {
                            candidate.server_revision != publication.server_revision
                        });
                    uploaded_any = true;
                } else {
                    let mut uploads = self.pending_uploads.borrow_mut();
                    let mut scheduler = self.scheduler.borrow_mut();
                    for key in current.keys() {
                        if let Some(ticket) = uploads.remove(key) {
                            let _ = scheduler.retry(ticket);
                        }
                    }
                    log_gpu_error("canonical cut allocation failed; complete cut requeued");
                }
            }

            let grouped = self
                .canonical_publications
                .borrow()
                .iter()
                .flat_map(|publication| publication.requirements.keys().copied())
                .collect::<BTreeSet<_>>();
            let individual = self
                .pending_uploads
                .borrow()
                .iter()
                .filter_map(|(key, ticket)| (!grouped.contains(key)).then_some((*key, *ticket)))
                .collect::<Vec<_>>();
            for (key, ticket) in individual {
                let ticket_current =
                    self.scheduler
                        .borrow()
                        .status(ticket.coord)
                        .is_some_and(|status| {
                            status.revision == ticket.revision
                                && status.state == ChunkState::Uploading
                        });
                let mesh_current = self
                    .pending_meshes
                    .borrow()
                    .get(&key)
                    .is_some_and(|mesh| mesh.revision == ticket.revision);
                if !ticket_current || !mesh_current {
                    self.pending_uploads.borrow_mut().remove(&key);
                    if ticket_current {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                    }
                    continue;
                }
                let uploaded = {
                    let chunks = self.chunks.borrow();
                    let meshes = self.pending_meshes.borrow();
                    chunks.get(&key).is_some_and(|chunk| {
                        meshes.get(&key).is_some_and(|mesh| {
                            self.renderer.borrow_mut().upload_chunk(chunk, &mesh.mesh)
                        })
                    })
                };
                if uploaded {
                    self.pending_uploads.borrow_mut().remove(&key);
                    self.pending_meshes.borrow_mut().remove(&key);
                    let _ = self.scheduler.borrow_mut().complete(ticket);
                    uploaded_any = true;
                } else {
                    self.pending_uploads.borrow_mut().remove(&key);
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                    log_gpu_error("voxel mesh arena allocation failed; upload requeued");
                }
            }
            uploaded_any
        }

        fn collision_stream_interest(
            &self,
            camera: &CameraState,
            streaming_velocity: Vec3,
            lookahead_seconds: f32,
        ) -> Vec<ChunkCoord> {
            crate::urgent_stream_interest(camera, streaming_velocity, lookahead_seconds)
        }

        fn movement_collision_interest(
            &self,
            camera: &CameraState,
            streaming_velocity: Vec3,
            lookahead_seconds: f32,
        ) -> Vec<ChunkCoord> {
            crate::movement_stream_interest(camera.position, streaming_velocity, lookahead_seconds)
                .into_iter()
                .collect()
        }

        fn enclosed_view_stream_plan(&self, camera: &CameraState) -> crate::EnclosedViewStreamPlan {
            let [width, height] = self.viewport_size.get();
            let Some(cone_tangent) = crate::viewport_view_cone_tangent(
                self.config.stream_view_cone_half_angle_degrees,
                crate::CAMERA_VERTICAL_FOV_RADIANS,
                width,
                height,
            ) else {
                return crate::EnclosedViewStreamPlan::default();
            };
            crate::enclosed_view_stream_plan(
                camera,
                self.config.stream_enclosed_view_distance_metres,
                cone_tangent,
                &self.chunk_portals.borrow(),
            )
        }

        fn enclosed_view_stream_interest(&self, camera: &CameraState) -> Vec<ChunkCoord> {
            self.enclosed_view_stream_plan(camera).chunks
        }

        fn submit_generation_batch(
            &self,
            priority: WorldProductPriority,
            tickets: Vec<voxels_runtime::WorkTicket>,
        ) {
            if tickets.is_empty() {
                return;
            }
            if let Err(error) = self.remote.submit_chunk_batch(priority, tickets.clone()) {
                for ticket in tickets {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                }
                if !matches!(
                    error,
                    RemoteWorldError::Backpressured
                        | RemoteWorldError::RequestWindowFull
                        | RemoteWorldError::NotOpen
                ) {
                    log_gpu_error(&format!("native world request failed: {error}"));
                }
            }
        }

        fn virtual_terrain_supported(&self) -> bool {
            self.remote.world_opened().is_some_and(|opened| {
                opened
                    .capabilities
                    .contains(WorldCapabilities::VIRTUAL_TERRAIN)
            })
        }

        fn virtual_terrain_view(&self, camera: &CameraState) -> VirtualTerrainView {
            let [width, height] = self.viewport_size.get();
            let height = height.max(1);
            let forward = camera.forward();
            VirtualTerrainView {
                camera_position_metres: camera.position.to_array().map(f64::from),
                camera_forward: forward.to_array().map(f64::from),
                vertical_fov_radians: f64::from(crate::CAMERA_VERTICAL_FOV_RADIANS),
                aspect_ratio: f64::from(width.max(1)) / f64::from(height),
                viewport_height_pixels: height,
                near_metres: 0.05,
                far_metres: f64::from(self.config.view_distance_metres),
                refine_above_pixels: VIRTUAL_TERRAIN_REFINE_ABOVE_PIXELS,
                coarsen_below_pixels: VIRTUAL_TERRAIN_COARSEN_BELOW_PIXELS,
                // Normal error is most visible on wet terrain. Always selecting against the
                // conservative wet bound prevents rain from changing geometry ownership.
                wet_specular_sensitivity: 1.0,
                force_exact_leaves: false,
            }
        }

        fn desired_virtual_terrain_columns(
            &self,
            camera: &CameraState,
            streaming_velocity: Vec3,
        ) -> Vec<[i32; 2]> {
            let camera_chunk = world_to_chunk(camera.position);
            let camera_leaf = TerrainPageKey::surface(0, camera_chunk.x, camera_chunk.z);
            let Some(camera_root) = camera_leaf.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL) else {
                return Vec::new();
            };
            let predicted_position = camera.position
                + streaming_velocity * self.config.stream_velocity_lookahead_seconds.max(0.0);
            let predicted_chunk = world_to_chunk(predicted_position);
            let predicted_root = TerrainPageKey::surface(0, predicted_chunk.x, predicted_chunk.z)
                .ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                .unwrap_or(camera_root);
            let current_column = [camera_root.coord[0], camera_root.coord[2]];
            let predicted_column = [predicted_root.coord[0], predicted_root.coord[2]];
            let mut prioritized = Vec::with_capacity(VIRTUAL_TERRAIN_MAX_COLUMNS);
            let mut included = BTreeSet::new();
            // Current ownership is needed immediately. The predicted endpoint follows so the
            // slowest build starts against its deadline, then every crossed fixed column is
            // enumerated explicitly. Pure endpoint ranking skipped the intervening 12.8 m roots
            // at flight speed and made successful lookahead useless.
            for column in std::iter::once(current_column)
                .chain(std::iter::once(predicted_column))
                .chain(virtual_terrain_column_corridor(
                    current_column,
                    predicted_column,
                ))
            {
                if included.insert(column) {
                    prioritized.push(column);
                    if prioritized.len() == VIRTUAL_TERRAIN_MAX_COLUMNS {
                        return prioritized;
                    }
                }
            }

            let camera_position = camera.position.to_array().map(f64::from);
            let predicted_position = predicted_position.to_array().map(f64::from);
            let forward = camera.forward().to_array().map(f64::from);
            let root_span_metres =
                f64::from(CHUNK_EDGE as u32 * (1_u32 << TERRAIN_COVERAGE_ROOT_LEVEL)) * 0.1;
            let radius = (f64::from(self.config.view_distance_metres) / root_span_metres)
                .ceil()
                .clamp(1.0, 64.0) as i32;
            // Only the remaining bounded request slots can be admitted. The directional score's
            // lead is two coverage columns, so the best remaining candidates are contained by a
            // radius derived from the square root of the slot count plus that lead and one
            // camera-within-column margin. Predicted and crossed columns outside this local search
            // were already reserved above.
            let ranking_radius =
                radius.min((VIRTUAL_TERRAIN_MAX_COLUMNS as f64).sqrt().ceil() as i32 + 2 + 1);
            let mut ranked = Vec::with_capacity(
                (ranking_radius.saturating_mul(2).saturating_add(1) as usize).pow(2),
            );
            for offset_z in -ranking_radius..=ranking_radius {
                for offset_x in -ranking_radius..=ranking_radius {
                    let column = [
                        camera_root.coord[0].saturating_add(offset_x),
                        camera_root.coord[2].saturating_add(offset_z),
                    ];
                    if included.contains(&column) {
                        continue;
                    }
                    let probe =
                        TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, column[0], column[1]);
                    if probe.horizontal_bounds().is_none() {
                        continue;
                    }
                    let center = terrain_page_center_metres(probe);
                    let delta_x = center[0] - camera_position[0];
                    let delta_z = center[2] - camera_position[2];
                    let current_distance_squared = delta_x * delta_x + delta_z * delta_z;
                    let predicted_delta_x = center[0] - predicted_position[0];
                    let predicted_delta_z = center[2] - predicted_position[2];
                    let predicted_distance_squared = predicted_delta_x * predicted_delta_x
                        + predicted_delta_z * predicted_delta_z;
                    let distance_squared = current_distance_squared.min(predicted_distance_squared);
                    if distance_squared
                        > f64::from(self.config.view_distance_metres).powi(2)
                            + root_span_metres.powi(2)
                    {
                        continue;
                    }
                    let forward_distance = delta_x * forward[0] + delta_z * forward[2];
                    // A two-column directional lead keeps fast travel supplied while the squared
                    // distance term guarantees that the camera-containing column ranks first.
                    let score =
                        distance_squared - forward_distance.max(0.0) * root_span_metres * 2.0;
                    ranked.push((column, score));
                }
            }
            let ranked_order = |left: &([i32; 2], f64), right: &([i32; 2], f64)| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.0.cmp(&right.0))
            };
            let remaining = VIRTUAL_TERRAIN_MAX_COLUMNS.saturating_sub(prioritized.len());
            let ranked_prefix = remaining.min(ranked.len());
            if ranked_prefix < ranked.len() {
                ranked.select_nth_unstable_by(ranked_prefix, ranked_order);
                ranked.truncate(ranked_prefix);
            }
            ranked.sort_by(ranked_order);
            prioritized.extend(ranked.into_iter().map(|(column, _)| column));
            prioritized
        }

        fn desired_virtual_terrain_roots(
            &self,
            prioritized_columns: &[[i32; 2]],
            camera: &CameraState,
        ) -> Vec<TerrainPageKey> {
            let state = self.virtual_terrain.borrow();
            let camera_position = camera.position.to_array().map(f64::from);
            let mut roots = Vec::new();
            for column in prioritized_columns
                .iter()
                .filter_map(|column| state.columns.get(column))
            {
                let mut column_roots = column.roots.to_vec();
                column_roots.sort_by(|left, right| {
                    terrain_page_distance_metres(*left, camera_position)
                        .total_cmp(&terrain_page_distance_metres(*right, camera_position))
                        .then_with(|| left.cmp(right))
                });
                roots.extend(column_roots);
                if roots.len() >= VIRTUAL_TERRAIN_MAX_REGIONS {
                    roots.truncate(VIRTUAL_TERRAIN_MAX_REGIONS);
                    break;
                }
            }
            roots
        }

        fn stream_virtual_terrain(&self, camera: &CameraState, streaming_velocity: Vec3) {
            if !self.virtual_terrain_supported() {
                return;
            }
            let now_ms = self.last_time.get().max(0.0) as u64;
            let prioritized_columns =
                self.desired_virtual_terrain_columns(camera, streaming_velocity);
            let desired_columns = prioritized_columns.iter().copied().collect::<BTreeSet<_>>();
            {
                let mut state = self.virtual_terrain.borrow_mut();
                let working_set = virtual_terrain_column_working_set(
                    &prioritized_columns,
                    state.columns.keys().copied(),
                    VIRTUAL_TERRAIN_COLUMN_WORKING_SET,
                );
                state
                    .columns
                    .retain(|column, _| working_set.contains(column));
                state
                    .column_retry_after_ms
                    .retain(|column, _| desired_columns.contains(column));
            }
            self.request_virtual_terrain_columns(&prioritized_columns, now_ms);

            let prioritized_roots =
                self.desired_virtual_terrain_roots(&prioritized_columns, camera);
            let camera_chunk = world_to_chunk(camera.position);
            let Some(camera_root) = TerrainPageKey::surface(0, camera_chunk.x, camera_chunk.z)
                .ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
            else {
                return;
            };
            let retained_roots = {
                let state = self.virtual_terrain.borrow();
                virtual_terrain_root_working_set(
                    &prioritized_roots,
                    state.registered_roots.iter().copied(),
                    camera_root,
                    VIRTUAL_TERRAIN_REGION_WORKING_SET,
                )
            };

            let removed_roots = {
                let state = self.virtual_terrain.borrow();
                state
                    .registered_roots
                    .difference(&retained_roots)
                    .copied()
                    .collect::<BTreeSet<_>>()
            };
            if !removed_roots.is_empty() {
                if let Err(error) = self
                    .renderer
                    .borrow_mut()
                    .retain_virtual_terrain_regions(retained_roots.iter().copied())
                {
                    log_gpu_error(&format!("retire virtual terrain regions: {error}"));
                } else {
                    let canceled = {
                        let mut state = self.virtual_terrain.borrow_mut();
                        let canceled = state
                            .directory_in_flight
                            .iter()
                            .filter(|(key, _)| {
                                key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                                    .is_some_and(|root| removed_roots.contains(&root))
                            })
                            .map(|(_, request_id)| *request_id)
                            .collect::<BTreeSet<_>>();
                        state
                            .directory_in_flight
                            .retain(|_, request_id| !canceled.contains(request_id));
                        state.directory_retry_after_ms.retain(|key, _| {
                            key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                                .is_none_or(|root| !removed_roots.contains(&root))
                        });
                        state
                            .registered_roots
                            .retain(|root| retained_roots.contains(root));
                        state.registered_refinements.retain(|key| {
                            key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                                .is_some_and(|root| retained_roots.contains(&root))
                        });
                        state.nodes.retain(|key, _| {
                            key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                                .is_some_and(|root| retained_roots.contains(&root))
                        });
                        canceled
                    };
                    for request_id in canceled {
                        self.remote.cancel(request_id);
                    }
                }
            }

            self.request_virtual_terrain_edit_directories(now_ms);
            self.request_virtual_terrain_directories(
                &prioritized_roots,
                now_ms,
                VIRTUAL_TERRAIN_MAX_DIRECTORY_BATCHES_IN_FLIGHT,
            );

            let view = self.virtual_terrain_view(camera);
            let mut cut = match self.renderer.borrow_mut().select_virtual_terrain_cut(view) {
                Ok(cut) => cut,
                Err(error) => {
                    log_gpu_error(&format!("select virtual terrain cut: {error}"));
                    return;
                }
            };
            if self.hydrate_virtual_terrain_from_cache(&cut) {
                match self.renderer.borrow_mut().select_virtual_terrain_cut(view) {
                    Ok(reselected) => cut = reselected,
                    Err(error) => {
                        log_gpu_error(&format!("reselect cached virtual terrain cut: {error}"));
                        return;
                    }
                }
            }

            if self
                .virtual_terrain
                .borrow()
                .minimum_region_revisions
                .is_empty()
                && self
                    .renderer
                    .borrow()
                    .virtual_terrain_candidate_is_gpu_certified()
            {
                // Install only one refinement-directory mutation at a time, and do not begin the
                // next until the GPU has certified the current CPU cut. Explicit pacing prevents
                // unbounded refinement churn without relying on transport contention.
                self.request_virtual_terrain_directories(
                    &cut.refinement_roots,
                    now_ms,
                    VIRTUAL_TERRAIN_MAX_REFINEMENT_DIRECTORY_BATCHES_IN_FLIGHT,
                );
            }

            let demands =
                self.virtual_terrain_demand_groups(&cut, view, streaming_velocity.length());
            if let Err(error) = self
                .virtual_terrain_scheduler
                .borrow_mut()
                .reconcile(demands)
            {
                log_gpu_error(&format!("reconcile virtual terrain demand: {error}"));
                return;
            }
            let batch = self
                .virtual_terrain_scheduler
                .borrow_mut()
                .next_batch(now_ms);
            if let Some(batch) = batch
                && let Err(error) = self.remote.submit_terrain_page_batch(
                    WorldProductPriority::VirtualTerrain,
                    batch.pages.clone(),
                )
            {
                let mut scheduler = self.virtual_terrain_scheduler.borrow_mut();
                for identity in batch.pages {
                    let _ = scheduler.fail(identity, now_ms);
                }
                if !matches!(
                    error,
                    RemoteWorldError::Backpressured
                        | RemoteWorldError::RequestWindowFull
                        | RemoteWorldError::NotOpen
                ) {
                    log_gpu_error(&format!("request virtual terrain pages: {error}"));
                }
            }

            if cut.is_renderable() {
                match self
                    .renderer
                    .borrow_mut()
                    .set_virtual_terrain_render_mode(VirtualTerrainRenderMode::Visible)
                {
                    Ok(()) => {}
                    Err(
                        VirtualTerrainRendererError::GpuCutNotCertified
                        | VirtualTerrainRendererError::NoRenderableCut
                        | VirtualTerrainRendererError::SelectedPageMissingGpu(_),
                    ) => {}
                    Err(error) => {
                        log_gpu_error(&format!("publish virtual terrain cut: {error}"));
                    }
                }
            }
        }

        fn request_virtual_terrain_columns(&self, desired_columns: &[[i32; 2]], now_ms: u64) {
            let columns = {
                let state = self.virtual_terrain.borrow();
                let in_flight_batches = state
                    .column_in_flight
                    .values()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len();
                if in_flight_batches >= VIRTUAL_TERRAIN_MAX_COLUMN_BATCHES_IN_FLIGHT {
                    return;
                }
                desired_columns
                    .iter()
                    .filter(|column| {
                        !state.columns.contains_key(*column)
                            && !state.column_in_flight.contains_key(*column)
                            && state
                                .column_retry_after_ms
                                .get(*column)
                                .is_none_or(|retry_at| *retry_at <= now_ms)
                    })
                    .take(VIRTUAL_TERRAIN_COLUMN_BATCH_SIZE)
                    .copied()
                    .collect::<Vec<_>>()
            };
            if columns.is_empty() {
                return;
            }
            match self.remote.submit_terrain_region_column_batch(
                WorldProductPriority::VirtualTerrain,
                columns.clone(),
            ) {
                Ok(request_id) => {
                    self.virtual_terrain
                        .borrow_mut()
                        .column_in_flight
                        .extend(columns.into_iter().map(|column| (column, request_id)));
                }
                Err(error) => {
                    let mut state = self.virtual_terrain.borrow_mut();
                    state.stats.column_submit_deferred =
                        state.stats.column_submit_deferred.saturating_add(1);
                    for column in columns {
                        state.column_retry_after_ms.insert(
                            column,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                    }
                    if !matches!(
                        error,
                        RemoteWorldError::Backpressured
                            | RemoteWorldError::RequestWindowFull
                            | RemoteWorldError::NotOpen
                    ) {
                        log_gpu_error(&format!("request virtual terrain columns: {error}"));
                    }
                }
            }
        }

        fn request_virtual_terrain_directories(
            &self,
            desired_roots: &[TerrainPageKey],
            now_ms: u64,
            maximum_in_flight_batches: usize,
        ) {
            let roots = {
                let state = self.virtual_terrain.borrow();
                let in_flight_batches = state
                    .directory_in_flight
                    .values()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len();
                let available = maximum_in_flight_batches.saturating_sub(in_flight_batches);
                desired_roots
                    .iter()
                    .filter(|root| {
                        !virtual_terrain_directory_is_registered(&state, **root)
                            && !state.directory_in_flight.contains_key(root)
                            && state
                                .directory_retry_after_ms
                                .get(root)
                                .is_none_or(|retry_at| *retry_at <= now_ms)
                    })
                    .take(available)
                    .copied()
                    .collect::<Vec<_>>()
            };
            if roots.is_empty() {
                return;
            }
            // One root per request lets the service's per-client generation lanes build distinct
            // deadlines concurrently. Putting two roots in one ordered batch serialized them
            // inside a single worker and made the second root miss fast-travel handoff.
            for root in roots {
                match self.remote.submit_terrain_directory_batch(
                    WorldProductPriority::VirtualTerrain,
                    vec![root],
                ) {
                    Ok(request_id) => {
                        self.virtual_terrain
                            .borrow_mut()
                            .directory_in_flight
                            .insert(root, request_id);
                    }
                    Err(error) => {
                        let mut state = self.virtual_terrain.borrow_mut();
                        state.stats.directory_submit_deferred =
                            state.stats.directory_submit_deferred.saturating_add(1);
                        state.directory_retry_after_ms.insert(
                            root,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                        drop(state);
                        if !matches!(
                            error,
                            RemoteWorldError::Backpressured
                                | RemoteWorldError::RequestWindowFull
                                | RemoteWorldError::NotOpen
                        ) {
                            log_gpu_error(&format!(
                                "request virtual terrain directory {root:?}: {error}"
                            ));
                        }
                        if matches!(
                            error,
                            RemoteWorldError::Backpressured
                                | RemoteWorldError::RequestWindowFull
                                | RemoteWorldError::NotOpen
                        ) {
                            break;
                        }
                    }
                }
            }
        }

        fn request_virtual_terrain_edit_directories(&self, now_ms: u64) {
            let roots = {
                let state = self.virtual_terrain.borrow();
                if state
                    .directory_in_flight
                    .values()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    >= VIRTUAL_TERRAIN_MAX_DIRECTORY_BATCHES_IN_FLIGHT
                {
                    return;
                }
                let mut roots = state
                    .minimum_region_revisions
                    .keys()
                    .filter(|root| {
                        !virtual_terrain_directory_is_registered(&state, **root)
                            && !state.directory_in_flight.contains_key(root)
                            && state
                                .directory_retry_after_ms
                                .get(root)
                                .is_none_or(|retry_at| *retry_at <= now_ms)
                    })
                    .copied()
                    .collect::<Vec<_>>();
                // The server can generate the complete spatial chain independently. Parent-first
                // order lets the completion install every refinement into the renderer in one
                // response instead of paying one network/GPU-feedback round trip per level.
                roots.sort_unstable_by(|left, right| {
                    right.level.cmp(&left.level).then_with(|| left.cmp(right))
                });
                roots.truncate(voxels_world::protocol::MAX_TERRAIN_DIRECTORIES_PER_BATCH);
                roots
            };
            if roots.is_empty() {
                return;
            }
            match self
                .remote
                .submit_terrain_directory_batch(WorldProductPriority::VirtualTerrain, roots.clone())
            {
                Ok(request_id) => {
                    self.virtual_terrain
                        .borrow_mut()
                        .directory_in_flight
                        .extend(roots.into_iter().map(|root| (root, request_id)));
                }
                Err(error) => {
                    let mut state = self.virtual_terrain.borrow_mut();
                    state.stats.directory_submit_deferred =
                        state.stats.directory_submit_deferred.saturating_add(1);
                    for root in roots {
                        state.directory_retry_after_ms.insert(
                            root,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                    }
                    drop(state);
                    if !matches!(
                        error,
                        RemoteWorldError::Backpressured
                            | RemoteWorldError::RequestWindowFull
                            | RemoteWorldError::NotOpen
                    ) {
                        log_gpu_error(&format!(
                            "request edited virtual terrain directory chain: {error}"
                        ));
                    }
                }
            }
        }

        fn hydrate_virtual_terrain_from_cache(&self, cut: &VirtualTerrainCut) -> bool {
            let mut uploaded = false;
            for identity in cut
                .requested_pages
                .iter()
                .take(VIRTUAL_TERRAIN_CACHE_UPLOADS_PER_FRAME)
            {
                let encoded = self
                    .virtual_terrain_cache
                    .borrow_mut()
                    .get_encoded(*identity);
                let Some(encoded) = encoded else {
                    continue;
                };
                let Ok(page) = decode_terrain_page(&encoded, self.source_identity_hash()) else {
                    continue;
                };
                match self.renderer.borrow_mut().upload_virtual_terrain_page(page) {
                    Ok(()) => uploaded = true,
                    Err(error) => {
                        log_gpu_error(&format!("upload cached virtual terrain page: {error}"));
                    }
                }
            }
            uploaded
        }

        fn virtual_terrain_demand_groups(
            &self,
            cut: &VirtualTerrainCut,
            view: VirtualTerrainView,
            speed_metres_per_second: f32,
        ) -> Vec<TerrainDemandGroup> {
            let state = self.virtual_terrain.borrow();
            let requested = cut
                .requested_pages
                .iter()
                .map(|identity| (identity.key, *identity))
                .collect::<BTreeMap<_, _>>();
            let mut grouped = BTreeSet::new();
            let mut groups = Vec::new();
            let parents = requested
                .keys()
                .filter_map(|key| key.parent())
                .collect::<BTreeSet<_>>();
            for parent in parents {
                let Some(children) = parent.refinement_children() else {
                    continue;
                };
                if !children.iter().all(|child| requested.contains_key(child)) {
                    continue;
                }
                let pages = children
                    .iter()
                    .filter_map(|child| {
                        let identity = requested.get(child)?;
                        let node = state.nodes.get(child)?;
                        Some(terrain_page_demand(
                            *identity,
                            node,
                            view,
                            speed_metres_per_second,
                        ))
                    })
                    .collect::<Vec<_>>();
                if pages.len() == children.len()
                    && let Ok(group) = TerrainDemandGroup::replacement(parent, pages)
                {
                    grouped.extend(children);
                    groups.push(group);
                }
            }
            for (key, identity) in requested {
                if grouped.contains(&key) {
                    continue;
                }
                let Some(node) = state.nodes.get(&key) else {
                    continue;
                };
                groups.push(TerrainDemandGroup::singleton(terrain_page_demand(
                    identity,
                    node,
                    view,
                    speed_metres_per_second,
                )));
            }
            groups
        }

        fn accept_remote_terrain_region_column_completion(
            &self,
            completion: RemoteTerrainRegionColumnCompletion,
        ) {
            let now_ms = self.last_time.get().max(0.0) as u64;
            let accepted = {
                let mut state = self.virtual_terrain.borrow_mut();
                let accepted = completion
                    .columns
                    .iter()
                    .copied()
                    .filter(|column| {
                        state.column_in_flight.get(column) == Some(&completion.request_id)
                    })
                    .collect::<BTreeSet<_>>();
                state.column_in_flight.retain(|column, request_id| {
                    *request_id != completion.request_id || !accepted.contains(column)
                });
                accepted
            };
            if accepted.is_empty() {
                return;
            }
            let result = match completion.result {
                Ok(result) => result,
                Err(error) => {
                    let mut state = self.virtual_terrain.borrow_mut();
                    match error {
                        RemoteWorldError::Preempted => {
                            state.stats.column_preempted =
                                state.stats.column_preempted.saturating_add(1);
                        }
                        RemoteWorldError::TimedOut => {
                            state.stats.column_timed_out =
                                state.stats.column_timed_out.saturating_add(1);
                        }
                        _ => {
                            state.stats.column_other_failed =
                                state.stats.column_other_failed.saturating_add(1);
                        }
                    }
                    for column in accepted {
                        state.column_retry_after_ms.insert(
                            column,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                    }
                    return;
                }
            };
            let mut state = self.virtual_terrain.borrow_mut();
            for item in result.items {
                if !accepted.contains(&item.column) {
                    continue;
                }
                let Ok(column) = item.result else {
                    state.stats.column_other_failed =
                        state.stats.column_other_failed.saturating_add(1);
                    state.column_retry_after_ms.insert(
                        item.column,
                        now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                    );
                    continue;
                };
                let minimum_revision = state
                    .minimum_column_revisions
                    .get(&item.column)
                    .copied()
                    .unwrap_or(0);
                if column.revision < minimum_revision {
                    state.stats.column_other_failed =
                        state.stats.column_other_failed.saturating_add(1);
                    state.column_retry_after_ms.insert(item.column, now_ms);
                    continue;
                }
                state.minimum_column_revisions.remove(&item.column);
                state.column_retry_after_ms.remove(&item.column);
                state.columns.insert(item.column, column);
                state.stats.column_accepted = state.stats.column_accepted.saturating_add(1);
            }
        }

        fn accept_remote_terrain_directory_completion(
            &self,
            completion: RemoteTerrainDirectoryCompletion,
        ) {
            let now_ms = self.last_time.get().max(0.0) as u64;
            let accepted = {
                let mut state = self.virtual_terrain.borrow_mut();
                let accepted = completion
                    .roots
                    .iter()
                    .copied()
                    .filter(|root| {
                        state.directory_in_flight.get(root) == Some(&completion.request_id)
                    })
                    .collect::<BTreeSet<_>>();
                for root in &accepted {
                    state.directory_in_flight.remove(root);
                }
                accepted
            };
            if accepted.is_empty() {
                return;
            }
            let result = match completion.result {
                Ok(result) => result,
                Err(error) => {
                    let preempted = matches!(error, RemoteWorldError::Preempted);
                    let mut state = self.virtual_terrain.borrow_mut();
                    match error {
                        RemoteWorldError::Preempted => {
                            state.stats.directory_preempted =
                                state.stats.directory_preempted.saturating_add(1);
                        }
                        RemoteWorldError::TimedOut => {
                            state.stats.directory_timed_out =
                                state.stats.directory_timed_out.saturating_add(1);
                        }
                        _ => {
                            state.stats.directory_other_failed =
                                state.stats.directory_other_failed.saturating_add(1);
                        }
                    }
                    for root in completion
                        .roots
                        .into_iter()
                        .filter(|root| accepted.contains(root))
                    {
                        state.directory_retry_after_ms.insert(
                            root,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                    }
                    drop(state);
                    // Collision-critical work is allowed to preempt disposable refinement. The
                    // still-published parent remains authoritative and the directory is retried,
                    // so surfacing this expected scheduling decision as a renderer error makes a
                    // healthy fast-travel frame look corrupt.
                    if !preempted {
                        log_gpu_error(&format!("virtual terrain directory batch failed: {error}"));
                    }
                    return;
                }
            };
            if result.source_identity_hash != self.source_identity_hash() {
                log_gpu_error("virtual terrain directory identity changed");
                return;
            }
            let mut items = result.items;
            items.sort_unstable_by(|left, right| {
                right
                    .root
                    .level
                    .cmp(&left.root.level)
                    .then_with(|| left.root.cmp(&right.root))
            });
            for item in items {
                let root = item.root;
                if !accepted.contains(&root) {
                    continue;
                }
                let is_refinement = root.is_surface() && root.level < TERRAIN_COVERAGE_ROOT_LEVEL;
                let bootstrap = match item.result {
                    Ok(bootstrap) => bootstrap,
                    Err(error) => {
                        let mut state = self.virtual_terrain.borrow_mut();
                        state.stats.directory_other_failed =
                            state.stats.directory_other_failed.saturating_add(1);
                        state.directory_retry_after_ms.insert(
                            root,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                        drop(state);
                        log_gpu_error(&format!(
                            "virtual terrain directory producer failed for {root:?}: {error:?}"
                        ));
                        continue;
                    }
                };
                let minimum_revision = self
                    .virtual_terrain
                    .borrow()
                    .minimum_region_revisions
                    .get(&root)
                    .copied()
                    .unwrap_or(0);
                let directory_revision = bootstrap
                    .directory
                    .nodes
                    .iter()
                    .find(|node| node.key == root && node.is_root)
                    .map_or(0, |node| node.revision);
                if directory_revision < minimum_revision {
                    let mut state = self.virtual_terrain.borrow_mut();
                    state.stats.directory_other_failed =
                        state.stats.directory_other_failed.saturating_add(1);
                    state.directory_retry_after_ms.insert(
                        root,
                        now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                    );
                    continue;
                }
                if virtual_terrain_directory_is_registered(&self.virtual_terrain.borrow(), root) {
                    continue;
                }
                let existing_roots = self
                    .virtual_terrain
                    .borrow()
                    .registered_roots
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                let publication = {
                    let mut renderer = self.renderer.borrow_mut();
                    if is_refinement {
                        renderer
                            .register_virtual_terrain_refinement_directory(&bootstrap.directory)
                            .map_err(|error| (error, None))
                    } else {
                        match renderer.register_virtual_terrain_directory(&bootstrap.directory) {
                            Ok(()) => match renderer
                                .upload_virtual_terrain_page(bootstrap.root_page.clone())
                            {
                                Ok(()) => Ok(()),
                                Err(error) => {
                                    let rollback = renderer.retain_virtual_terrain_regions(
                                        existing_roots.iter().copied(),
                                    );
                                    Err((error, rollback.err()))
                                }
                            },
                            Err(error) => Err((error, None)),
                        }
                    }
                };
                match publication {
                    Ok(()) => {
                        if !is_refinement {
                            match encode_terrain_page(&bootstrap.root_page) {
                                Ok(encoded) => {
                                    if let Err(error) = self
                                        .virtual_terrain_cache
                                        .borrow_mut()
                                        .insert(encoded, false)
                                    {
                                        log_gpu_error(&format!(
                                            "cache virtual terrain bootstrap root: {error}"
                                        ));
                                    }
                                }
                                Err(error) => {
                                    log_gpu_error(&format!(
                                        "encode virtual terrain bootstrap root: {error}"
                                    ));
                                }
                            }
                        }
                        let mut state = self.virtual_terrain.borrow_mut();
                        if is_refinement {
                            state.registered_refinements.insert(root);
                        } else {
                            state.registered_roots.insert(root);
                        }
                        state.stats.directory_accepted =
                            state.stats.directory_accepted.saturating_add(1);
                        state.directory_retry_after_ms.remove(&root);
                        state.minimum_region_revisions.remove(&root);
                        for mut node in bootstrap.directory.nodes {
                            if is_refinement {
                                node.is_root = false;
                            }
                            if let Some(existing) = state.nodes.get_mut(&node.key) {
                                existing.has_children |= node.has_children;
                            } else {
                                state.nodes.insert(node.key, node);
                            }
                        }
                    }
                    Err((error, rollback_error)) => {
                        let mut state = self.virtual_terrain.borrow_mut();
                        state.stats.directory_other_failed =
                            state.stats.directory_other_failed.saturating_add(1);
                        state.directory_retry_after_ms.insert(
                            root,
                            now_ms.saturating_add(VIRTUAL_TERRAIN_DIRECTORY_RETRY_MS),
                        );
                        drop(state);
                        log_gpu_error(&format!(
                            "register virtual terrain directory {root:?}: {error}"
                        ));
                        if let Some(rollback_error) = rollback_error {
                            log_gpu_error(&format!(
                                "rollback virtual terrain bootstrap: {rollback_error}"
                            ));
                        }
                    }
                }
            }
        }

        fn invalidate_virtual_terrain_regions(
            &self,
            affected_chunks: &[ChunkCoord],
            minimum_revision: Option<u64>,
        ) {
            let (invalid_roots, revision_keys) =
                crate::virtual_terrain_edit_revision_keys(affected_chunks);
            if invalid_roots.is_empty() {
                return;
            }
            let invalid_columns = invalid_roots
                .iter()
                .map(|root| [root.coord[0], root.coord[2]])
                .collect::<BTreeSet<_>>();
            let keep = {
                let state = self.virtual_terrain.borrow();
                state
                    .registered_roots
                    .difference(&invalid_roots)
                    .copied()
                    .collect::<BTreeSet<_>>()
            };
            let canceled_requests = {
                let mut state = self.virtual_terrain.borrow_mut();
                let request_ids = state
                    .directory_in_flight
                    .iter()
                    .filter(|(key, _)| {
                        key.level < TERRAIN_COVERAGE_ROOT_LEVEL
                            || key
                                .ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                                .is_some_and(|root| invalid_roots.contains(&root))
                    })
                    .map(|(_, request_id)| *request_id)
                    .chain(
                        invalid_columns
                            .iter()
                            .filter_map(|column| state.column_in_flight.get(column).copied()),
                    )
                    .collect::<BTreeSet<_>>();
                if !request_ids.is_empty() {
                    let canceled_roots = state
                        .directory_in_flight
                        .iter()
                        .filter(|(_, request_id)| request_ids.contains(request_id))
                        .map(|(root, _)| *root)
                        .collect::<Vec<_>>();
                    for root in canceled_roots {
                        state.directory_in_flight.remove(&root);
                        state.directory_retry_after_ms.remove(&root);
                    }
                    let canceled_columns = state
                        .column_in_flight
                        .iter()
                        .filter(|(_, request_id)| request_ids.contains(request_id))
                        .map(|(column, _)| *column)
                        .collect::<Vec<_>>();
                    for column in canceled_columns {
                        state.column_in_flight.remove(&column);
                        state.column_retry_after_ms.remove(&column);
                    }
                }
                request_ids
            };
            for request_id in canceled_requests {
                self.remote.cancel(request_id);
            }
            if let Err(error) = self
                .renderer
                .borrow_mut()
                .retain_virtual_terrain_regions(keep.iter().copied())
            {
                log_gpu_error(&format!(
                    "invalidate edited virtual terrain regions: {error}"
                ));
                return;
            }
            let mut state = self.virtual_terrain.borrow_mut();
            for column in invalid_columns {
                state.columns.remove(&column);
                state.column_retry_after_ms.remove(&column);
                if let Some(revision) = minimum_revision {
                    state
                        .minimum_column_revisions
                        .entry(column)
                        .and_modify(|minimum| *minimum = (*minimum).max(revision))
                        .or_insert(revision);
                }
            }
            state
                .registered_roots
                .retain(|root| !invalid_roots.contains(root));
            state.registered_refinements.retain(|key| {
                key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                    .is_none_or(|root| !invalid_roots.contains(&root))
            });
            state.nodes.retain(|key, _| {
                key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                    .is_none_or(|root| !invalid_roots.contains(&root))
            });
            state.directory_retry_after_ms.retain(|key, _| {
                key.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                    .is_none_or(|root| !invalid_roots.contains(&root))
            });
            for key in revision_keys {
                if let Some(revision) = minimum_revision {
                    state
                        .minimum_region_revisions
                        .entry(key)
                        .and_modify(|minimum| *minimum = (*minimum).max(revision))
                        .or_insert(revision);
                }
            }
        }

        fn reset_virtual_terrain_streaming(&self) {
            let request_ids = {
                let state = self.virtual_terrain.borrow();
                state
                    .directory_in_flight
                    .values()
                    .chain(state.column_in_flight.values())
                    .copied()
                    .collect::<BTreeSet<_>>()
            };
            for request_id in request_ids {
                self.remote.cancel(request_id);
            }
            if let Err(error) = self
                .renderer
                .borrow_mut()
                .retain_virtual_terrain_regions(std::iter::empty())
            {
                log_gpu_error(&format!("reset virtual terrain regions: {error}"));
            }
            *self.virtual_terrain.borrow_mut() = VirtualTerrainStreamingState::default();
            if let Err(error) = self
                .virtual_terrain_scheduler
                .borrow_mut()
                .reconcile(std::iter::empty())
            {
                log_gpu_error(&format!("reset virtual terrain scheduler: {error}"));
            }
        }

        fn accept_remote_terrain_page_completion(&self, completion: RemoteTerrainPageCompletion) {
            let now_ms = self.last_time.get().max(0.0) as u64;
            let Ok(result) = completion.result else {
                let mut scheduler = self.virtual_terrain_scheduler.borrow_mut();
                for identity in completion.requested {
                    let _ = scheduler.fail(identity, now_ms);
                }
                return;
            };
            for item in result.batch.items {
                let identity = item.requested;
                let page = match item.result {
                    Ok(page) => page,
                    Err(_) => {
                        let _ = self
                            .virtual_terrain_scheduler
                            .borrow_mut()
                            .fail(identity, now_ms);
                        continue;
                    }
                };
                let encoded = match encode_terrain_page(&page) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        let _ = self
                            .virtual_terrain_scheduler
                            .borrow_mut()
                            .fail(identity, now_ms);
                        log_gpu_error(&format!("encode received virtual terrain page: {error}"));
                        continue;
                    }
                };
                let useful = self
                    .virtual_terrain_scheduler
                    .borrow_mut()
                    .complete(identity, encoded.len())
                    .unwrap_or(false);
                if let Err(error) = self
                    .virtual_terrain_cache
                    .borrow_mut()
                    .insert(encoded, false)
                {
                    log_gpu_error(&format!("cache virtual terrain page: {error}"));
                }
                let still_registered = self
                    .virtual_terrain
                    .borrow()
                    .nodes
                    .get(&identity.key)
                    .is_some_and(|node| {
                        node.revision == identity.revision
                            && node.content_fingerprint == identity.content_fingerprint
                    });
                if useful
                    && still_registered
                    && let Err(error) = self.renderer.borrow_mut().upload_virtual_terrain_page(page)
                {
                    log_gpu_error(&format!("upload virtual terrain page: {error}"));
                }
            }
        }

        fn drain_remote_generation(&self) {
            if let Some(completion) = self.remote.next_completion() {
                self.accept_remote_completion(completion);
            }
            for _ in 0..VIRTUAL_TERRAIN_MAX_COLUMN_BATCHES_IN_FLIGHT {
                let Some(completion) = self.remote.next_terrain_region_column_completion() else {
                    break;
                };
                self.accept_remote_terrain_region_column_completion(completion);
            }
            for _ in 0..VIRTUAL_TERRAIN_MAX_DIRECTORY_BATCHES_IN_FLIGHT {
                let Some(completion) = self.remote.next_terrain_directory_completion() else {
                    break;
                };
                self.accept_remote_terrain_directory_completion(completion);
            }
            for _ in 0..VIRTUAL_TERRAIN_PAGE_COMPLETIONS_PER_FRAME {
                let Some(completion) = self.remote.next_terrain_page_completion() else {
                    break;
                };
                self.accept_remote_terrain_page_completion(completion);
            }
        }

        fn accept_remote_completion(&self, completion: RemoteChunkCompletion) {
            let Ok(result) = completion.result else {
                for ticket in completion.tickets {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                }
                return;
            };
            if result.source_identity_hash != self.source_identity_hash() {
                for ticket in completion.tickets {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                }
                log_gpu_error("native world response identity changed");
                return;
            }
            let mut items = result.items;
            for ticket in completion.tickets {
                let Some(index) = items.iter().position(|item| item.coord == ticket.coord) else {
                    let _ = self.scheduler.borrow_mut().retry(ticket);
                    continue;
                };
                let item = items.remove(index);
                match item.result {
                    Ok(snapshot) => {
                        self.accept_generated_chunk(ticket, item.edit_revision, snapshot)
                    }
                    Err(voxels_world::WorldSourceError::SourceCoverageUnavailable) => {
                        // This source owns finite coverage. Leaving the exact scheduler capability
                        // in flight forms a conservative collision boundary without retry thrash;
                        // focus eviction releases it normally.
                        log_gpu_error(&format!(
                            "native world has no coverage for chunk {:?}",
                            ticket.coord
                        ));
                    }
                    Err(error) => {
                        let _ = self.scheduler.borrow_mut().retry(ticket);
                        log_gpu_error(&format!(
                            "native world could not generate chunk {:?}: {error}",
                            ticket.coord
                        ));
                    }
                }
            }
        }

        fn accept_generated_chunk(
            &self,
            ticket: voxels_runtime::WorkTicket,
            edit_revision: u64,
            snapshot: voxels_world::ChunkSnapshot,
        ) {
            if snapshot.source_identity_hash != self.source_identity_hash()
                || snapshot.chunk.coord() != ticket.coord
                || snapshot.meshing_halo.coord() != ticket.coord
            {
                let _ = self.scheduler.borrow_mut().retry(ticket);
                return;
            }
            let server_floor = self.edit_revisions.borrow().chunk_floor(ticket.coord);
            if !revision_satisfies(edit_revision, server_floor) {
                let _ = self.scheduler.borrow_mut().retry(ticket);
                return;
            }
            // Network completions can arrive after focus/edit invalidation. The scheduler
            // capability is the admission check; stale bytes never attach to a newer revision.
            if self.scheduler.borrow_mut().complete(ticket) != CompletionStatus::Accepted {
                return;
            }
            let key = coord_key(ticket.coord);
            self.chunk_portals
                .borrow_mut()
                .insert(key, ChunkPortalMask::from_chunk(&snapshot.chunk));
            self.chunks.borrow_mut().insert(key, snapshot.chunk);
            self.chunk_halos
                .borrow_mut()
                .insert(key, snapshot.meshing_halo);
        }

        fn reconcile_chunk_activation(
            &self,
            focus: ChunkCoord,
            collision_interest: &[ChunkCoord],
            enclosed_view_interest: &[ChunkCoord],
            enclosed_view_frontiers: &[crate::PortalFrontier],
            surface_interest: &[ChunkCoord],
        ) {
            let scheduler = self.scheduler.borrow();
            let config = scheduler.config();
            let mut radial = BTreeSet::new();
            for dz in -config.load_radius_chunks..=config.load_radius_chunks {
                for dx in -config.load_radius_chunks..=config.load_radius_chunks {
                    if i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)
                        > i64::from(config.load_radius_chunks)
                            * i64::from(config.load_radius_chunks)
                    {
                        continue;
                    }
                    let Some(x) = focus.x.checked_add(dx) else {
                        continue;
                    };
                    let Some(z) = focus.z.checked_add(dz) else {
                        continue;
                    };
                    let column: Vec<_> = (-config.vertical_radius_chunks
                        ..=config.vertical_radius_chunks)
                        .filter_map(|dy| focus.y.checked_add(dy).map(|y| ChunkCoord::new(x, y, z)))
                        .collect();
                    if column
                        .iter()
                        .all(|coord| scheduler.desired_chunk_renderable(*coord))
                    {
                        radial.extend(column.into_iter().map(coord_key));
                    }
                }
            }
            // Preserve only exact, complete, active 3D sets. The renderer must not use an inactive
            // retained Y profile to suppress a surface parent for a different vertical band.
            let mut canonical_ready_chunks = radial.clone();
            // Preserve the old radial reason for retained resident meshes until the scheduler
            // actually evicts them. This carries visible coverage across small focus moves while
            // new columns become atomically ready, matching the retention hysteresis contract.
            for key in self.radial_active_chunks.borrow().iter().copied() {
                if scheduler
                    .status(ChunkCoord::new(key.0, key.1, key.2))
                    .is_some()
                {
                    radial.insert(key);
                }
            }

            let complete_interest = |interest: &[ChunkCoord]| {
                crate::complete_renderable_interest_columns(interest, |coord| {
                    scheduler.desired_chunk_renderable(coord)
                })
            };
            let interaction = complete_interest(collision_interest);
            // Exact cave/tunnel chunks supplement the surface hierarchy independently in 3D.
            // Do not apply the surface cut's all-Y-siblings rule here: portal discovery can add a
            // pending chunk to an already visible column, and that must not revoke its ready wall.
            let enclosed_view =
                crate::renderable_exact_interest_chunks(enclosed_view_interest, |coord| {
                    scheduler.desired_chunk_renderable(coord)
                });
            let frontier_faces =
                crate::exact_volume_frontier_faces(enclosed_view_frontiers, |coord| {
                    scheduler.desired_chunk_renderable(coord)
                })
                .into_iter()
                .map(
                    |frontier| voxels_render::renderer::ExactVolumeFrontierFace {
                        chunk: frontier.chunk,
                        face: frontier.face,
                        cells: frontier.cells,
                    },
                )
                .collect::<Vec<_>>();
            let surface = complete_interest(surface_interest);
            canonical_ready_chunks.extend(surface.iter().copied());
            let canonical_surface_ready_chunks = surface.clone();
            drop(scheduler);
            self.reconcile_activation_reason(
                &self.radial_active_chunks,
                radial,
                ChunkActivationReason::Radial,
            );
            self.reconcile_activation_reason(
                &self.interaction_active_chunks,
                interaction,
                ChunkActivationReason::Interaction,
            );
            self.reconcile_activation_reason(
                &self.enclosed_view_active_chunks,
                enclosed_view.clone(),
                ChunkActivationReason::EnclosedView,
            );
            self.reconcile_activation_reason(
                &self.surface_active_chunks,
                surface,
                ChunkActivationReason::Surface,
            );
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_canonical_cut_ready_chunks(
                canonical_ready_chunks,
                canonical_surface_ready_chunks,
            );
            renderer.set_enclosed_view_ready_chunks(enclosed_view);
            renderer.set_exact_volume_frontier_faces(&frontier_faces);
        }

        fn reconcile_activation_reason(
            &self,
            current: &RefCell<BTreeSet<(i32, i32, i32)>>,
            next: BTreeSet<(i32, i32, i32)>,
            reason: ChunkActivationReason,
        ) {
            let mut current = current.borrow_mut();
            let removed: Vec<_> = current.difference(&next).copied().collect();
            let added: Vec<_> = next.difference(&current).copied().collect();
            *current = next;
            drop(current);
            if removed.is_empty() && added.is_empty() {
                return;
            }
            let mut renderer = self.renderer.borrow_mut();
            for (x, y, z) in removed {
                renderer.set_chunk_activation(ChunkCoord::new(x, y, z), reason, false);
            }
            for (x, y, z) in added {
                renderer.set_chunk_activation(ChunkCoord::new(x, y, z), reason, true);
            }
        }

        async fn stop(&self) {
            self.prepare_stop();
            let camera = *self.camera.borrow();
            self.presence.close_after_final_pose(&camera).await;
            self.remote.close();
        }

        fn stop_now(&self) {
            self.prepare_stop();
            self.presence.close();
            self.remote.close();
        }

        fn prepare_stop(&self) {
            if let Some(restore) = self.profile_restore_camera.take() {
                *self.camera.borrow_mut() = restore;
            }
            self.reproduction_camera.set(None);
            if let Some(restore) = self.reproduction_restore_camera.take() {
                *self.camera.borrow_mut() = restore;
            }
            self.stopped.set(true);
            let id = self.frame_id.replace(0);
            if id != 0 {
                let _ = self.scope.cancel_animation_frame(id);
            }
            self.callback.borrow_mut().take();
        }

        fn spectator_available(&self) -> bool {
            self.config.developer_controls_enabled
                && self.remote.world_opened().is_some_and(|opened| {
                    opened
                        .capabilities
                        .contains(WorldCapabilities::SPECTATOR_MODE)
                })
        }

        fn gliding_available(&self) -> bool {
            self.remote
                .world_opened()
                .is_some_and(|opened| opened.capabilities.contains(WorldCapabilities::GLIDING))
        }

        fn apply_renderer_host_ui_action(&self) {
            let action = self.renderer.borrow_mut().take_host_ui_action();
            let Some(HostUiAction::SpectatorRequested(requested)) = action else {
                return;
            };
            self.set_spectator(requested);
        }

        fn set_spectator(&self, requested: bool) -> bool {
            let active = requested && self.spectator_available();
            self.input.borrow_mut().clear();
            let mut camera = self.camera.borrow_mut();
            let was_active = camera.locomotion() == LocomotionMode::Spectator;
            if active && !was_active {
                self.spectator_body.set(Some(*camera));
                camera.set_locomotion(LocomotionMode::Spectator);
            } else if !active && was_active {
                if let Some(body) = self.spectator_body.take() {
                    *camera = body;
                } else {
                    camera.set_locomotion(LocomotionMode::Walking);
                }
            }
            let active = camera.locomotion() == LocomotionMode::Spectator;
            self.presence.send_pose_now(&camera, self.last_time.get());
            drop(camera);
            self.renderer.borrow_mut().set_spectator_active(active);
            active
        }

        fn feed_input(&self, bytes: &[u8]) -> bool {
            for chunk in bytes.chunks_exact(INPUT_RECORD_SIZE) {
                let record = bytemuck::pod_read_unaligned::<InputRecord>(chunk);
                match record.kind {
                    KIND_POINTER_DOWN => {
                        if self
                            .renderer
                            .borrow()
                            .edit_shape_control_contains(record.x, record.y)
                        {
                            self.renderer
                                .borrow_mut()
                                .handle_ui_pointer_down(record.x, record.y);
                            continue;
                        }
                        if record.code == 1
                            && self
                                .renderer
                                .borrow()
                                .inventory_wheel_contains(record.x, record.y)
                        {
                            self.touch_inventory_drag.set(Some([record.x, record.y]));
                            continue;
                        }
                        if record.code == 1 {
                            self.touch_inventory_drag.set(None);
                        }
                        let was_open = self.renderer.borrow().ui_open();
                        // Mission Control has no secondary-click behavior. Ignore right clicks
                        // while it is open instead of turning them into accidental button presses.
                        let is_open = if was_open && record.buttons & 2 != 0 {
                            true
                        } else {
                            self.renderer
                                .borrow_mut()
                                .handle_ui_pointer_down(record.x, record.y)
                        };
                        self.apply_renderer_host_ui_action();
                        if !was_open && is_open {
                            self.input.borrow_mut().clear();
                        }
                        if !was_open && !is_open {
                            self.edit_target(record.buttons);
                        }
                    }
                    KIND_POINTER_MOVE => {
                        if record.code == 1
                            && let Some(anchor) = self.touch_inventory_drag.get()
                        {
                            if let Some((steps, next_anchor)) =
                                crate::inventory_swipe(anchor, [record.x, record.y])
                            {
                                self.touch_inventory_drag.set(Some(next_anchor));
                                let direction = steps.signum();
                                for _ in 0..steps.unsigned_abs() {
                                    let _ = self
                                        .renderer
                                        .borrow_mut()
                                        .cycle_placement_material(direction);
                                }
                            }
                            continue;
                        }
                        if self.renderer.borrow().ui_open() {
                            self.renderer
                                .borrow_mut()
                                .handle_ui_pointer_move(record.x, record.y);
                        } else {
                            self.camera
                                .borrow_mut()
                                .look(Vec2::new(record.dx, record.dy));
                        }
                    }
                    KIND_WHEEL => {
                        if self.renderer.borrow().ui_open() {
                            continue;
                        }
                        let direction = if record.dy >= 0.0 { 1 } else { -1 };
                        let _ = self
                            .renderer
                            .borrow_mut()
                            .cycle_placement_material(direction);
                    }
                    KIND_POINTER_UP => {
                        if record.code == 1 {
                            self.touch_inventory_drag.set(None);
                        }
                    }
                    KIND_KEY_DOWN => {
                        if record.code == 8 {
                            let was_open = self.renderer.borrow().ui_open();
                            let is_open = self.renderer.borrow_mut().handle_ui_key(
                                record.code,
                                true,
                                record.flags & 1 != 0,
                            );
                            if !was_open && is_open {
                                self.input.borrow_mut().clear();
                            }
                        } else if record.code == 19 && record.flags & 1 == 0 {
                            self.renderer.borrow_mut().request_screenshot();
                        } else if !self.renderer.borrow().ui_open()
                            && record.flags & 1 == 0
                            && record.code == 7
                        {
                            self.renderer.borrow_mut().cycle_edit_shape();
                        } else if !self.renderer.borrow().ui_open()
                            && record.flags & 1 == 0
                            && (9..=18).contains(&record.code)
                        {
                            self.renderer
                                .borrow_mut()
                                .select_placement_slot(usize::from(record.code - 9));
                        } else if !self.renderer.borrow().ui_open() {
                            self.input.borrow_mut().set_key(record.code, true);
                        }
                    }
                    KIND_KEY_UP => {
                        if record.code == 8 {
                            self.renderer
                                .borrow_mut()
                                .handle_ui_key(record.code, false, false);
                        } else if record.code != 7
                            && record.code != 19
                            && !(9..=18).contains(&record.code)
                        {
                            self.input.borrow_mut().set_key(record.code, false);
                        }
                    }
                    KIND_CANCEL => {
                        self.touch_inventory_drag.set(None);
                        self.input.borrow_mut().clear();
                    }
                    _ => {}
                }
            }
            self.renderer.borrow().ui_open()
        }

        fn edit_target(&self, buttons: u16) {
            let camera = *self.camera.borrow();
            let hit = self.raycast_target(&camera);
            let Some(hit) = hit else {
                return;
            };
            let shape = self.renderer.borrow().edit_shape();
            let action = if buttons & 1 != 0 {
                let hit_coord = VoxelCoord::new(hit.voxel[0], hit.voxel[1], hit.voxel[2]);
                let Some(volume) = EditVolume::for_hit(hit_coord, shape) else {
                    return;
                };
                if !self.dig_volume_resident(volume) {
                    self.renderer
                        .borrow_mut()
                        .show_gameplay_toast("Loading edit area…");
                    return;
                }
                EditAction::Dig {
                    hit: hit_coord,
                    shape,
                }
            } else if buttons & 2 != 0 {
                let surface = VoxelCoord::new(hit.voxel[0], hit.voxel[1], hit.voxel[2]);
                let Some(volume) = EditVolume::for_placement(surface, hit.normal, shape) else {
                    return;
                };
                if volume.coordinates().any(|coord| {
                    camera.intersects_voxel([coord.x, coord.y, coord.z], VOXEL_SIZE_METRES)
                }) {
                    self.renderer
                        .borrow_mut()
                        .show_gameplay_toast("Cannot place through your body");
                    return;
                }
                if !self.dig_volume_resident(volume) {
                    self.renderer
                        .borrow_mut()
                        .show_gameplay_toast("Loading edit area…");
                    return;
                }
                let placement_material = { self.renderer.borrow().placement_material() };
                let Some(placement_material) = placement_material else {
                    self.renderer
                        .borrow_mut()
                        .show_gameplay_toast("Dig material before placing");
                    return;
                };
                EditAction::Place {
                    coord: volume.centre(),
                    material: placement_material,
                    shape,
                }
            } else {
                return;
            };

            // The server expands the anchor to the shared metre-scale stencil and atomically owns
            // material yield/debit. The browser never emits independently raceable mutations.
            let _ = self.submit_local_edit(action);
        }

        fn apply_server_edits(&self) {
            for event in self.remote.drain_edit_events() {
                match event {
                    RemoteEditEvent::Commit(commit) => {
                        if let Some(inventory) = commit.editor_inventory {
                            self.inventory.set(inventory);
                            self.renderer
                                .borrow_mut()
                                .set_inventory_counts(inventory.counts);
                        }
                        self.apply_durable_edits(
                            &commit.mutations,
                            commit.revision,
                            &commit.affected_chunks,
                        );
                    }
                    RemoteEditEvent::ResyncRequired { revision } => {
                        self.resynchronize_world_products(revision);
                    }
                    RemoteEditEvent::Rejected {
                        operation_id,
                        message,
                    } => {
                        log_gpu_error(&format!(
                            "server rejected edit operation {operation_id}: {message}"
                        ));
                        self.renderer.borrow_mut().show_gameplay_toast(message);
                    }
                }
            }
        }

        fn submit_local_edit(&self, action: EditAction) -> [usize; 2] {
            if self.camera.borrow().locomotion() == LocomotionMode::Spectator {
                return [0, 0];
            }
            match self.remote.submit_edit(action) {
                Ok(_) => [1, 0],
                Err(error) => {
                    log_gpu_error(&format!("submit authoritative edit: {error}"));
                    [0, 0]
                }
            }
        }

        fn apply_durable_edits(
            &self,
            mutations: &[VoxelMutation],
            server_revision: u64,
            affected_chunks: &[ChunkCoord],
        ) -> EditRequirements {
            if mutations.is_empty() {
                return EditRequirements::default();
            }
            let coords = mutations
                .iter()
                .map(|mutation| mutation.coord)
                .collect::<Vec<_>>();
            let apply_values = self.edit_revisions.borrow_mut().observe_commit_batch(
                &coords,
                server_revision,
                affected_chunks,
            );
            let accepted_mutations = mutations
                .iter()
                .copied()
                .zip(apply_values)
                .filter_map(|(mutation, apply)| apply.then_some(mutation))
                .collect::<Vec<_>>();
            {
                let mut edits = self.edits.borrow_mut();
                let changes = accepted_mutations
                    .iter()
                    .map(|mutation| (mutation.coord, Some(mutation.material)))
                    .collect::<Vec<_>>();
                edits.replace_durable_overrides(&changes);
            }
            voxels_world::apply_resident_mutations(
                &mut self.chunks.borrow_mut(),
                &mut self.chunk_halos.borrow_mut(),
                &accepted_mutations,
            );
            if !accepted_mutations.is_empty() {
                let mut changed_chunks = BTreeMap::<ChunkCoord, (Vec<[usize; 3]>, bool)>::new();
                for mutation in &accepted_mutations {
                    let entry = changed_chunks
                        .entry(mutation.coord.chunk())
                        .or_insert_with(|| (Vec::new(), false));
                    entry.0.push(mutation.coord.local());
                    entry.1 |= mutation.material.occludes_ambient();
                }
                let chunks = self.chunks.borrow();
                let mut portals = self.chunk_portals.borrow_mut();
                for (coord, (local_voxels, added_occluder)) in changed_chunks {
                    let key = coord_key(coord);
                    if let Some(chunk) = chunks.get(&key) {
                        if !added_occluder && let Some(mask) = portals.get_mut(&key) {
                            mask.add_non_occluding_voxels(chunk, &local_voxels);
                        } else {
                            // Adding an occluder can split a component, which requires a fresh
                            // connected-component analysis. This path is intentionally exact.
                            portals.insert(key, ChunkPortalMask::from_chunk(chunk));
                        }
                    }
                }
                self.last_enclosure_probe.set(f64::NEG_INFINITY);
                self.invalidate_virtual_terrain_regions(affected_chunks, Some(server_revision));
            }
            let canonical: Vec<CanonicalRequirement> = {
                let mut scheduler = self.scheduler.borrow_mut();
                let report = scheduler.mark_voxels_edited(&coords);
                report
                    .affected_chunks
                    .into_iter()
                    .filter_map(|coord| {
                        let status = scheduler.status(coord)?;
                        status.desired.then_some(CanonicalRequirement {
                            coord,
                            revision: status.revision,
                        })
                    })
                    .collect()
            };
            self.register_canonical_publication(server_revision, &canonical);
            let performance = self.scope.performance();
            let started_ms = performance_now(performance.as_ref());
            let requirements = EditRequirements { canonical };
            let mut trackers = self.edit_trackers.borrow_mut();
            let target = mutations[0].coord;
            if let Some(index) = trackers.iter().position(|tracker| tracker.target == target) {
                trackers.remove(index);
            }
            if trackers.len() == self.config.max_edit_trackers {
                trackers.pop_front();
            }
            trackers.push_back(EditTracker {
                target,
                started_ms,
                requirements: EditRequirements {
                    canonical: requirements.canonical.clone(),
                },
            });
            requirements
        }

        fn resynchronize_world_products(&self, revision: u64) {
            log_gpu_error(&format!(
                "edit stream overflowed at revision {revision}; reconciling retained world products"
            ));
            *self.edits.borrow_mut() = EditMap::default();
            self.edit_revisions.borrow_mut().clear();
            self.pending_meshes.borrow_mut().clear();
            self.pending_uploads.borrow_mut().clear();
            self.canonical_publications.borrow_mut().clear();
            self.scheduler.borrow_mut().invalidate_all_generation();
            self.reset_virtual_terrain_streaming();
        }

        fn update_edit_convergence(&self, now_ms: f64, submitted: bool) {
            if !submitted || self.edit_trackers.borrow().is_empty() {
                return;
            }
            let scheduler = self.scheduler.borrow();
            let mut trackers = self.edit_trackers.borrow_mut();
            let mut pending = VecDeque::with_capacity(trackers.len());
            while let Some(tracker) = trackers.pop_front() {
                let canonical_ready = tracker.requirements.canonical.iter().all(|requirement| {
                    scheduler.status(requirement.coord).is_none_or(|status| {
                        !status.desired
                            || (status.state == ChunkState::Resident
                                && revision_satisfies(status.revision, requirement.revision))
                    })
                });
                if canonical_ready {
                    let full_ms = (now_ms - tracker.started_ms) as f32;
                    self.edit_last_ms.set(full_ms);
                } else {
                    pending.push_back(tracker);
                }
            }
            *trackers = pending;
        }

        fn raycast_target(&self, camera: &CameraState) -> Option<VoxelHit> {
            let chunks = self.chunks.borrow();
            let camera_voxel = VoxelCoord::new(
                (camera.position.x / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.y / VOXEL_SIZE_METRES).floor() as i32,
                (camera.position.z / VOXEL_SIZE_METRES).floor() as i32,
            );
            let mut skipping_origin_water =
                resident_material(&chunks, camera_voxel) == Some(Material::Water);
            raycast_voxels(
                camera.position,
                camera.forward(),
                5.0,
                VOXEL_SIZE_METRES,
                |x, y, z| {
                    let coord = VoxelCoord::new(x, y, z);
                    let material = resident_material(&chunks, coord);
                    if skipping_origin_water && material == Some(Material::Water) {
                        false
                    } else {
                        skipping_origin_water = false;
                        material.is_some_and(|material| {
                            material.is_collidable() || material == Material::Water
                        })
                    }
                },
            )
        }

        fn dig_target(
            &self,
            camera: &CameraState,
            shape: EditShape,
        ) -> Option<(VoxelHit, EditVolume)> {
            let hit = self.raycast_target(camera)?;
            let volume = EditVolume::for_hit(
                VoxelCoord::new(hit.voxel[0], hit.voxel[1], hit.voxel[2]),
                shape,
            )?;
            self.dig_volume_resident(volume).then_some((hit, volume))
        }

        fn dig_volume_resident(&self, volume: EditVolume) -> bool {
            let minimum = volume.min.chunk();
            let maximum = volume.max.chunk();
            let chunks = self.chunks.borrow();
            for z in minimum.z..=maximum.z {
                for y in minimum.y..=maximum.y {
                    for x in minimum.x..=maximum.x {
                        if !chunks.contains_key(&(x, y, z)) {
                            return false;
                        }
                    }
                }
            }
            true
        }
    }

    #[wasm_bindgen]
    pub struct MissionControlScreenshot {
        filename: String,
        metadata: String,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        terrain_diagnostic_u32x5: Vec<u8>,
    }

    impl From<ScreenshotCapture> for MissionControlScreenshot {
        fn from(capture: ScreenshotCapture) -> Self {
            Self {
                filename: capture.filename,
                metadata: capture.metadata,
                width: capture.width,
                height: capture.height,
                rgba: capture.rgba,
                terrain_diagnostic_u32x5: capture.terrain_diagnostic_u32x5,
            }
        }
    }

    #[wasm_bindgen]
    impl MissionControlScreenshot {
        #[wasm_bindgen(getter)]
        pub fn filename(&self) -> String {
            self.filename.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn metadata(&self) -> String {
            self.metadata.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn width(&self) -> u32 {
            self.width
        }

        #[wasm_bindgen(getter)]
        pub fn height(&self) -> u32 {
            self.height
        }

        pub fn rgba(&mut self) -> Vec<u8> {
            std::mem::take(&mut self.rgba)
        }

        pub fn terrain_diagnostic_u32x5(&mut self) -> Vec<u8> {
            std::mem::take(&mut self.terrain_diagnostic_u32x5)
        }
    }

    #[wasm_bindgen]
    pub struct EngineHandle {
        engine: Option<Rc<Engine>>,
    }

    #[wasm_bindgen]
    impl EngineHandle {
        pub fn automation_contract(&self) -> String {
            format!(
                "{AUTOMATION_CONTRACT_VERSION}\n{SNAPSHOT_SCHEMA_VERSION}\n\
                 {FRAME_SAMPLE_WIDTH}\n{GPU_SAMPLE_WIDTH}\n\
                 {PLAYER_EYE_HEIGHT_METRES},{PLAYER_HEIGHT_METRES},{PLAYER_RADIUS_METRES},\
                 {EDIT_CUBE_EDGE_VOXELS},{EDIT_CUBE_VOLUME_VOXELS},\
                 {EDIT_SPHERE_RADIUS_VOXELS},{EDIT_SPHERE_VOLUME_VOXELS}\n\
                 {SNAPSHOT_FIELD_NAMES}"
            )
        }

        /// Applies one embedded `voxels.reproduction.v2` document atomically. An empty return
        /// value means success; a non-empty value explains the exact identity or state mismatch.
        pub fn apply_reproduction(&self, metadata: &str) -> String {
            let Some(engine) = self.engine.as_ref() else {
                return "engine is unavailable".to_owned();
            };
            engine
                .apply_reproduction(metadata)
                .err()
                .unwrap_or_default()
        }

        pub fn clear_reproduction(&self) {
            if let Some(engine) = self.engine.as_ref() {
                engine.clear_reproduction();
            }
        }

        pub fn start_profile(&self, profile_id: u32) -> bool {
            self.engine
                .as_ref()
                .is_some_and(|engine| engine.start_profile(profile_id))
        }

        /// Enters or leaves the same server-authorized spectator role exposed by World Lab.
        /// Returning restores the exact local body snapshot captured on entry.
        pub fn set_spectator(&self, active: bool) -> bool {
            self.engine
                .as_ref()
                .is_some_and(|engine| engine.set_spectator(active))
        }

        pub fn feed_input(&self, bytes: &[u8]) -> bool {
            if let Some(engine) = self.engine.as_ref() {
                engine.feed_input(bytes)
            } else {
                false
            }
        }

        pub fn ui_open(&self) -> bool {
            self.engine
                .as_ref()
                .is_some_and(|engine| engine.renderer.borrow().ui_open())
        }

        pub fn take_mission_control_copy(&self) -> Option<String> {
            self.engine
                .as_ref()
                .and_then(|engine| engine.renderer.borrow_mut().take_diagnostics_copy())
        }

        pub fn report_mission_control_copy_result(&self, copied: bool) {
            if let Some(engine) = self.engine.as_ref() {
                engine
                    .renderer
                    .borrow_mut()
                    .report_diagnostics_copy_result(copied);
            }
        }

        pub fn mission_control_screenshot_pending(&self) -> bool {
            self.engine
                .as_ref()
                .is_some_and(|engine| engine.renderer.borrow().screenshot_pending())
        }

        pub fn take_mission_control_screenshot(&self) -> Option<MissionControlScreenshot> {
            self.engine
                .as_ref()
                .and_then(|engine| engine.renderer.borrow_mut().take_screenshot_capture())
                .map(Into::into)
        }

        pub fn report_mission_control_screenshot_result(&self, saved: bool) {
            if let Some(engine) = self.engine.as_ref() {
                engine.renderer.borrow_mut().report_screenshot_result(saved);
            }
        }

        pub fn resize(&self, css_width: f32, css_height: f32, dpr: f32) {
            if let Some(engine) = self.engine.as_ref() {
                let width = (css_width * dpr).round().max(1.0) as u32;
                let height = (css_height * dpr).round().max(1.0) as u32;
                engine.viewport_size.set([width, height]);
                engine.renderer.borrow_mut().resize(width, height, dpr);
            }
        }

        pub fn set_reduced_motion(&self, reduced_motion: bool) {
            if let Some(engine) = self.engine.as_ref() {
                engine
                    .renderer
                    .borrow_mut()
                    .set_reduced_motion(reduced_motion);
            }
        }

        pub fn set_diagnostic_sky(&self, enabled: bool, red: u8, green: u8, blue: u8) -> bool {
            let Some(engine) = self.engine.as_ref() else {
                return false;
            };
            let color = enabled
                .then(|| [red, green, blue].map(|channel| f32::from(channel) / f32::from(u8::MAX)));
            engine.renderer.borrow_mut().set_diagnostic_sky_color(color);
            true
        }

        pub fn set_geometry_source_debug(&self, enabled: bool) -> bool {
            let Some(engine) = self.engine.as_ref() else {
                return false;
            };
            engine
                .renderer
                .borrow_mut()
                .set_geometry_source_debug(enabled);
            true
        }

        pub fn set_material_detail(&self, enabled: bool) -> bool {
            let Some(engine) = self.engine.as_ref() else {
                return false;
            };
            engine
                .renderer
                .borrow_mut()
                .set_material_detail_enabled(enabled);
            true
        }

        /// Reports actual current/outgoing cut ownership for one canonical voxel. This is a
        /// read-only automation assertion over renderer state, not an alternate streaming path.
        pub fn exact_volume_presented(&self, voxel_x: i32, voxel_y: i32, voxel_z: i32) -> bool {
            self.engine.as_ref().is_some_and(|engine| {
                engine
                    .renderer
                    .borrow()
                    .edited_chunk_presented(VoxelCoord::new(voxel_x, voxel_y, voxel_z).chunk())
            })
        }

        /// `[resident, required, playable]` for the browser's canvas-only startup surface.
        pub fn startup_progress(&self) -> Vec<u32> {
            let Some(engine) = self.engine.as_ref() else {
                return vec![0, 0, 0];
            };
            let readiness = engine
                .scheduler
                .borrow()
                .vicinity_readiness(engine.config.startup_ready_radius_chunks);
            vec![
                usize_to_u32(readiness.resident),
                usize_to_u32(readiness.required),
                u32::from(engine.startup_ready.get()),
            ]
        }

        /// Deterministic browser-harness seam that submits through the same server-authoritative
        /// path as pointer input. It does not mutate local world state optimistically.
        pub fn submit_place(&self, x: i32, y: i32, z: i32, material_id: u16, shape_id: u8) -> bool {
            let Some(engine) = self.engine.as_ref() else {
                return false;
            };
            let Some(material) = Material::from_id(material_id) else {
                return false;
            };
            let Some(shape) = automation_edit_shape(shape_id) else {
                return false;
            };
            engine.submit_local_edit(EditAction::Place {
                coord: VoxelCoord::new(x, y, z),
                material,
                shape,
            })[0]
                == 1
        }

        /// Deterministic browser-harness seam for the exact gameplay dig action. The server, not
        /// this API, expands the hit voxel into the selected one-cubic-metre brush and validates
        /// reach.
        pub fn submit_dig(&self, x: i32, y: i32, z: i32, shape_id: u8) -> bool {
            self.engine.as_ref().is_some_and(|engine| {
                let Some(shape) = automation_edit_shape(shape_id) else {
                    return false;
                };
                engine.submit_local_edit(EditAction::Dig {
                    hit: VoxelCoord::new(x, y, z),
                    shape,
                })[0]
                    == 1
            })
        }

        /// `[inventory_revision, air, grass, ..., glow_crystal]` in stable material-ID order.
        pub fn inventory(&self) -> Vec<f64> {
            let Some(engine) = self.engine.as_ref() else {
                return Vec::new();
            };
            let inventory = engine.inventory.get();
            std::iter::once(inventory.revision as f64)
                .chain(inventory.counts.into_iter().map(|count| count as f64))
                .collect()
        }

        pub fn snapshot(&self) -> Vec<f32> {
            let mut values = Vec::new();
            if let Some(engine) = self.engine.as_ref() {
                let camera = engine.camera.borrow();
                let fluid = camera.fluid_state();
                let diagnostics = engine.scheduler.borrow().diagnostics();
                let profile = *engine.profile.borrow();
                let camera_voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
                let camera_voxel_y = (camera.position.y / VOXEL_SIZE_METRES).floor() as i32;
                let camera_voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
                let (render, target, canonical_lattice_presented, terrain_column_ownership) = {
                    let renderer = engine.renderer.borrow();
                    (
                        renderer.diagnostics(),
                        renderer.target_voxel(),
                        renderer.canonical_lattice_presented(
                            camera_voxel_x,
                            camera_voxel_y,
                            camera_voxel_z,
                        ),
                        renderer.terrain_column_ownership_at(camera_voxel_x, camera_voxel_z),
                    )
                };
                let streaming_velocity = if profile.running() {
                    camera.velocity
                } else {
                    camera.streaming_velocity(&engine.input.borrow())
                };
                let exact_streaming_velocity =
                    crate::exact_streaming_velocity(&camera, streaming_velocity);
                let collision_immediate_interest =
                    engine.movement_collision_interest(&camera, exact_streaming_velocity, 0.1);
                let collision_lookahead_seconds =
                    (engine.config.stream_collision_lookahead_seconds
                        - crate::COLLISION_READINESS_RESERVE_SECONDS)
                        .max(0.1);
                let collision_lookahead_interest = engine.movement_collision_interest(
                    &camera,
                    exact_streaming_velocity,
                    collision_lookahead_seconds,
                );
                let enclosed_view_interest = engine.enclosed_view_stream_interest(&camera);
                let (
                    canonical_immediate,
                    collision_immediate,
                    collision_lookahead,
                    enclosed_view,
                    enclosed_view_renderable,
                ) = {
                    let scheduler = engine.scheduler.borrow();
                    (
                        scheduler.vicinity_readiness_at(world_to_chunk(camera.position), 1),
                        scheduler.interest_readiness(&collision_immediate_interest),
                        scheduler.interest_readiness(&collision_lookahead_interest),
                        scheduler.interest_readiness(&enclosed_view_interest),
                        enclosed_view_interest
                            .iter()
                            .filter(|coord| scheduler.desired_chunk_renderable(**coord))
                            .count(),
                    )
                };
                let enclosed_view_owned = {
                    let renderer = engine.renderer.borrow();
                    enclosed_view_interest
                        .iter()
                        .filter(|coord| renderer.enclosed_view_chunk_owned(**coord))
                        .count()
                };
                let edit_canonical_coords = engine
                    .edit_trackers
                    .borrow()
                    .iter()
                    .flat_map(|tracker| {
                        tracker
                            .requirements
                            .canonical
                            .iter()
                            .map(|requirement| requirement.coord)
                    })
                    .collect::<BTreeSet<_>>();
                let edit_canonical_renderable = {
                    let scheduler = engine.scheduler.borrow();
                    edit_canonical_coords
                        .iter()
                        .filter(|coord| {
                            scheduler.status(**coord).is_none_or(|status| {
                                !status.desired || status.state == ChunkState::Resident
                            })
                        })
                        .count()
                };
                let edit_canonical_owned = {
                    let renderer = engine.renderer.borrow();
                    edit_canonical_coords
                        .iter()
                        .filter(|coord| renderer.edited_chunk_presented(**coord))
                        .count()
                };
                let canonical_voxel_bytes = engine
                    .chunks
                    .borrow()
                    .len()
                    .saturating_mul(CHUNK_VOXEL_BYTES)
                    .saturating_add(
                        engine
                            .chunk_halos
                            .borrow()
                            .values()
                            .map(MeshingHalo::logical_bytes)
                            .sum::<usize>(),
                    );
                let pending_mesh_bytes = engine
                    .pending_meshes
                    .borrow()
                    .values()
                    .map(|pending| pending.mesh.retained_bytes())
                    .sum::<usize>();
                let edit_logical_bytes = engine.edits.borrow().logical_bytes();
                let stream_interest = engine.cinder_stream_interest.get();
                let stream_interest_keys: BTreeSet<_> = stream_interest
                    .as_slice()
                    .iter()
                    .copied()
                    .map(coord_key)
                    .collect();
                let portal_active = engine.portal_active_chunks.borrow();
                let portal_active_columns: BTreeSet<_> =
                    portal_active.iter().map(|(x, _, z)| (*x, *z)).collect();
                let unreachable_portal_active = portal_active
                    .iter()
                    .filter(|key| !stream_interest_keys.contains(key))
                    .count();
                let (
                    virtual_terrain_mode,
                    virtual_terrain_resident_pages,
                    virtual_terrain_resident_bytes,
                    virtual_terrain_resident_primitives,
                ) = {
                    let renderer = engine.renderer.borrow();
                    let (pages, bytes, primitives, _, _) = renderer.virtual_terrain_usage();
                    let mode = match renderer.virtual_terrain_render_mode() {
                        VirtualTerrainRenderMode::Disabled => 0.0,
                        VirtualTerrainRenderMode::Shadow => 1.0,
                        VirtualTerrainRenderMode::Visible => 2.0,
                    };
                    (mode, pages, bytes, primitives)
                };
                let (
                    virtual_terrain_columns,
                    virtual_terrain_column_in_flight,
                    virtual_terrain_column_revision_floors,
                    virtual_terrain_current_column_known,
                    virtual_terrain_current_column_roots,
                    virtual_terrain_current_column_registered_roots,
                    virtual_terrain_nearest_registered_root_metres,
                    virtual_terrain_registered_regions,
                    virtual_terrain_directory_in_flight,
                    virtual_terrain_directory_nodes,
                    virtual_terrain_column_accepted,
                    virtual_terrain_column_submit_deferred,
                    virtual_terrain_column_preempted,
                    virtual_terrain_column_timed_out,
                    virtual_terrain_column_other_failed,
                    virtual_terrain_directory_accepted,
                    virtual_terrain_directory_submit_deferred,
                    virtual_terrain_directory_preempted,
                    virtual_terrain_directory_timed_out,
                    virtual_terrain_directory_other_failed,
                ) = {
                    let state = engine.virtual_terrain.borrow();
                    let camera_chunk = world_to_chunk(camera.position);
                    let current_column = TerrainPageKey::surface(0, camera_chunk.x, camera_chunk.z)
                        .ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                        .map(|root| [root.coord[0], root.coord[2]]);
                    let current_roots =
                        current_column.and_then(|column| state.columns.get(&column));
                    let current_registered_roots = current_roots.map_or(0, |column| {
                        column
                            .roots
                            .iter()
                            .filter(|root| state.registered_roots.contains(root))
                            .count()
                    });
                    let camera_position = camera.position.to_array().map(f64::from);
                    let nearest_registered_root_metres = state
                        .registered_roots
                        .iter()
                        .map(|root| terrain_page_distance_metres(*root, camera_position))
                        .reduce(f64::min)
                        .unwrap_or(-1.0);
                    (
                        state.columns.len(),
                        state.column_in_flight.len(),
                        state.minimum_column_revisions.len(),
                        usize::from(current_roots.is_some()),
                        current_roots.map_or(0, |column| column.roots.len()),
                        current_registered_roots,
                        nearest_registered_root_metres,
                        state.registered_roots.len(),
                        state.directory_in_flight.len(),
                        state.nodes.len(),
                        state.stats.column_accepted,
                        state.stats.column_submit_deferred,
                        state.stats.column_preempted,
                        state.stats.column_timed_out,
                        state.stats.column_other_failed,
                        state.stats.directory_accepted,
                        state.stats.directory_submit_deferred,
                        state.stats.directory_preempted,
                        state.stats.directory_timed_out,
                        state.stats.directory_other_failed,
                    )
                };
                let virtual_terrain_stream = engine.virtual_terrain_scheduler.borrow().stats();
                let (virtual_terrain_cache_pages, virtual_terrain_cache_bytes) = {
                    let cache = engine.virtual_terrain_cache.borrow();
                    (cache.len(), cache.resident_bytes())
                };
                values.extend_from_slice(&[
                    camera.position.x,
                    camera.position.y,
                    camera.position.z,
                    camera.yaw,
                    camera.pitch,
                    if camera.grounded { 1.0 } else { 0.0 },
                    engine.renderer.borrow().quad_count() as f32,
                    engine.edits.borrow().len() as f32,
                    diagnostics.resident as f32,
                    diagnostics.tracked as f32,
                    render.visible_chunks as f32,
                    render.draw_calls as f32,
                    render.arena_pages as f32,
                    render.arena_allocated_bytes as f32 / (1024.0 * 1024.0),
                    render.arena_capacity_bytes as f32 / (1024.0 * 1024.0),
                    (diagnostics.generation.queued
                        + diagnostics.generation.in_flight
                        + diagnostics.meshing.queued
                        + diagnostics.meshing.in_flight
                        + diagnostics.upload.queued
                        + diagnostics.upload.in_flight) as f32,
                    engine.frame_milliseconds.get(),
                    render.shadow_draw_calls as f32,
                    render.shadow_cascades as f32,
                    diagnostics.initial_residency_latency.p95_frames as f32,
                    diagnostics.initial_residency_latency.max_frames as f32,
                    diagnostics.remesh_latency.p95_frames as f32,
                    diagnostics.remesh_latency.max_frames as f32,
                    render.water_quads as f32,
                    render.water_draw_calls as f32,
                    render.refraction_copy_bytes as f32 / (1024.0 * 1024.0),
                    fluid.immersion,
                    fluid.eye_depth_metres,
                    if fluid.eyes_submerged { 1.0 } else { 0.0 },
                    if fluid.swimming { 1.0 } else { 0.0 },
                    target.map_or(0.0, |coord| coord[0] as f32),
                    target.map_or(0.0, |coord| coord[1] as f32),
                    target.map_or(0.0, |coord| coord[2] as f32),
                    if target.is_some() { 1.0 } else { 0.0 },
                    render.core_gpu_bytes as f32 / (1024.0 * 1024.0),
                    engine.cpu_milliseconds.get(),
                    engine.simulation_milliseconds.get(),
                    engine.stream_milliseconds.get(),
                    engine.render_milliseconds.get(),
                    render.gpu_sample_id as f32,
                    render.gpu_total_ms.unwrap_or(-1.0),
                    render.gpu_shadow_ms.unwrap_or(-1.0),
                    render.gpu_world_ms.unwrap_or(-1.0),
                    render.gpu_water_ms.unwrap_or(-1.0),
                    render.gpu_ui_ms.unwrap_or(-1.0),
                    wasm_committed_bytes() as f32 / (1024.0 * 1024.0),
                    canonical_voxel_bytes as f32 / (1024.0 * 1024.0),
                    pending_mesh_bytes as f32 / (1024.0 * 1024.0),
                    edit_logical_bytes as f32 / (1024.0 * 1024.0),
                    diagnostics.total_evictions as f32,
                    diagnostics.stale_completions as f32,
                    profile.phase() as u8 as f32,
                    profile.elapsed_seconds(),
                    profile.distance_metres(),
                    if profile.phase() == ProfilePhase::Complete {
                        1.0
                    } else {
                        0.0
                    },
                    engine.profile_tracked_high.get() as f32,
                    engine.profile_pending_high.get() as f32,
                    engine.profile_pending_mesh_high.get() as f32,
                    engine.profile_arena_capacity_high.get() as f32 / (1024.0 * 1024.0),
                    engine.profile_wasm_high.get() as f32 / (1024.0 * 1024.0),
                    diagnostics
                        .total_evictions
                        .saturating_sub(engine.profile_start_evictions.get())
                        as f32,
                    if render.material_detail { 1.0 } else { 0.0 },
                    render.daylight_phase as f32,
                    render.surface_region as f32,
                    render.cloud_coverage,
                    if render.screen_space_ambient_occlusion {
                        1.0
                    } else {
                        0.0
                    },
                    render.gpu_depth_prepass_ms.unwrap_or(-1.0),
                    render.gpu_ambient_occlusion_ms.unwrap_or(-1.0),
                    render.ambient_occlusion_bytes as f32 / (1024.0 * 1024.0),
                    render.depth_prepass_draw_calls as f32,
                    render.enclosure,
                    render.interior_exposure,
                    if render.cave_headlamp { 1.0 } else { 0.0 },
                    engine.enclosure_probe_microseconds.get(),
                    render.local_light_candidates as f32,
                    render.active_local_lights as f32,
                    render.clipped_local_lights as f32,
                    render.occluded_local_lights as f32,
                    render.portal_rejected_local_lights as f32,
                    render.local_light_visibility_tests as f32,
                    engine
                        .cinder_portal_state
                        .get()
                        .open_count(CINDER_VAULT_PORTAL_COUNT) as f32,
                    engine.cinder_portal_revision.get() as f32,
                    if render.local_lighting { 1.0 } else { 0.0 },
                    engine
                        .renderer
                        .borrow()
                        .placement_material()
                        .unwrap_or(Material::Air)
                        .id() as f32,
                    diagnostics.secondary_interest_requested as f32,
                    diagnostics.secondary_interest_normalized as f32,
                    diagnostics.secondary_interest_desired as f32,
                    diagnostics.secondary_interest_truncated as f32,
                    if stream_interest.overflowed() {
                        1.0
                    } else {
                        0.0
                    },
                    portal_active.len() as f32,
                    portal_active_columns.len() as f32,
                    unreachable_portal_active as f32,
                    render.remote_avatars as f32,
                    render.avatar_parts as f32,
                    render.avatar_draw_calls as f32,
                    (render.viewport_fingerprint & 0x00ff_ffff) as f32,
                    ((render.viewport_fingerprint >> 24) & 0x00ff_ffff) as f32,
                    if engine.terrain_ready.get() { 1.0 } else { 0.0 },
                    render.cpu_cull_ms,
                    render.cpu_encode_ms,
                    render.cpu_submit_ms,
                    render.draw_list_tested_slices as f32,
                    render.draw_list_selected_slices as f32,
                    render.surface_width as f32,
                    render.surface_height as f32,
                    render.dpr,
                    render.day_fraction,
                    render.local_solar_day_fraction,
                    render.year_fraction,
                    render.moon_orbit_fraction,
                    render.twinkle_phase,
                    render.latitude_degrees,
                    render.longitude_degrees,
                    render.local_sidereal_angle_radians,
                    render.moon_illuminated_fraction,
                    render.celestial_revision as f32,
                    render.sun_direction[0],
                    render.sun_direction[1],
                    render.sun_direction[2],
                    render.moon_direction[0],
                    render.moon_direction[1],
                    render.moon_direction[2],
                    render.shadow_strength,
                    render.cloud_offset_metres[0],
                    render.cloud_offset_metres[1],
                    render.cloud_velocity_metres_per_second[0],
                    render.cloud_velocity_metres_per_second[1],
                    render.weather_revision as f32,
                    render.weather_kind as f32,
                    render.weather_fraction,
                    render.precipitation,
                    render.storminess,
                    render.lightning,
                    render.cloud_density,
                    render.cloud_base_metres,
                    render.cloud_top_metres,
                    render.cloud_render_resolution[0] as f32,
                    render.cloud_render_resolution[1] as f32,
                    render.cloud_steps[0] as f32,
                    render.cloud_steps[1] as f32,
                    render.fog_density,
                    render.outdoor_exposure,
                    if camera.locomotion() == LocomotionMode::Spectator {
                        1.0
                    } else {
                        0.0
                    },
                    if canonical_lattice_presented {
                        1.0
                    } else {
                        0.0
                    },
                    canonical_immediate.resident as f32,
                    canonical_immediate.required as f32,
                    f32::from(terrain_column_ownership.0),
                    f32::from(terrain_column_ownership.1),
                    diagnostics.generation.queued as f32,
                    diagnostics.generation.in_flight as f32,
                    diagnostics.meshing.queued as f32,
                    diagnostics.meshing.in_flight as f32,
                    diagnostics.upload.queued as f32,
                    diagnostics.upload.in_flight as f32,
                    diagnostics.initial_residency_latency.completed as f32,
                    diagnostics.initial_residency_latency.in_flight as f32,
                    diagnostics.accepted_completions as f32,
                    collision_immediate.resident as f32,
                    collision_immediate.required as f32,
                    collision_lookahead.resident as f32,
                    collision_lookahead.required as f32,
                    collision_lookahead_seconds,
                    edit_canonical_coords.len() as f32,
                    edit_canonical_renderable as f32,
                    edit_canonical_owned as f32,
                    enclosed_view.resident as f32,
                    enclosed_view.required as f32,
                    enclosed_view_renderable as f32,
                    enclosed_view_owned as f32,
                    virtual_terrain_mode,
                    virtual_terrain_registered_regions as f32,
                    virtual_terrain_directory_in_flight as f32,
                    virtual_terrain_directory_nodes as f32,
                    virtual_terrain_resident_pages as f32,
                    virtual_terrain_resident_bytes as f32 / (1024.0 * 1024.0),
                    virtual_terrain_resident_primitives as f32,
                    render.virtual_terrain_gpu_selected_pages as f32,
                    render.virtual_terrain_gpu_requested_pages as f32,
                    render.virtual_terrain_published_ownerless_roots as f32,
                    if render.virtual_terrain_gpu_matches_cpu_cut {
                        1.0
                    } else {
                        0.0
                    },
                    render.virtual_terrain_gpu_overflow_flags as f32,
                    render.virtual_terrain_gpu_stack_peak as f32,
                    render.virtual_terrain_gpu_ownerless_roots as f32,
                    virtual_terrain_stream.pending_pages as f32,
                    virtual_terrain_stream.in_flight_pages as f32,
                    virtual_terrain_stream.cancellation_waste_bytes as f32 / (1024.0 * 1024.0),
                    virtual_terrain_cache_pages as f32,
                    virtual_terrain_cache_bytes as f32 / (1024.0 * 1024.0),
                    virtual_terrain_columns as f32,
                    virtual_terrain_column_in_flight as f32,
                    virtual_terrain_column_revision_floors as f32,
                    virtual_terrain_current_column_known as f32,
                    virtual_terrain_current_column_roots as f32,
                    virtual_terrain_current_column_registered_roots as f32,
                    virtual_terrain_nearest_registered_root_metres as f32,
                    virtual_terrain_column_accepted as f32,
                    virtual_terrain_column_submit_deferred as f32,
                    virtual_terrain_column_preempted as f32,
                    virtual_terrain_column_timed_out as f32,
                    virtual_terrain_column_other_failed as f32,
                    virtual_terrain_directory_accepted as f32,
                    virtual_terrain_directory_submit_deferred as f32,
                    virtual_terrain_directory_preempted as f32,
                    virtual_terrain_directory_timed_out as f32,
                    virtual_terrain_directory_other_failed as f32,
                    render.virtual_terrain_published_pages as f32,
                    render.virtual_terrain_published_exact_pages as f32,
                    render.virtual_terrain_published_minimum_level as f32,
                    render.virtual_terrain_published_maximum_level as f32,
                    (render.virtual_terrain_cut_fingerprint & 0x00ff_ffff) as f32,
                    ((render.virtual_terrain_cut_fingerprint >> 24) & 0x00ff_ffff) as f32,
                    engine.frame_sequence.get() as f32,
                    SNAPSHOT_SCHEMA_VERSION as f32,
                ]);
                engine.frame_history.borrow_mut().drain_into(&mut values);
                let gpu_timings = engine.renderer.borrow_mut().drain_gpu_timings();
                values.push(gpu_timings.samples.len() as f32);
                values.push(gpu_timings.dropped as f32);
                for sample in gpu_timings.samples {
                    values.extend_from_slice(&[
                        sample.frame_id as f32,
                        sample.total_ms,
                        sample.shadow_ms,
                        sample.shadow_cascade_ms[0],
                        sample.shadow_cascade_ms[1],
                        sample.shadow_cascade_ms[2],
                        sample.depth_prepass_ms,
                        sample.ambient_occlusion_ms,
                        sample.world_ms,
                        sample.water_ms,
                        sample.cloud_ms,
                        sample.weather_ms,
                        sample.ui_ms,
                        sample.virtual_terrain_traversal_ms,
                        sample.virtual_terrain_compaction_ms,
                    ]);
                }
            }
            values
        }

        pub async fn destroy(&mut self) {
            if let Some(engine) = self.engine.take() {
                engine.stop().await;
            }
        }
    }

    const fn automation_edit_shape(shape_id: u8) -> Option<EditShape> {
        match shape_id {
            0 => Some(EditShape::Sphere),
            1 => Some(EditShape::Cube),
            _ => None,
        }
    }

    impl Drop for EngineHandle {
        fn drop(&mut self) {
            if let Some(engine) = self.engine.take() {
                engine.stop_now();
            }
        }
    }

    #[wasm_bindgen]
    pub async fn create_engine(
        canvas: OffscreenCanvas,
        css_width: f32,
        css_height: f32,
        dpr: f32,
        reduced_motion: bool,
        config_toml: String,
        player: js_sys::Array,
    ) -> Result<EngineHandle, JsValue> {
        console_error_panic_hook::set_once();
        if player.length() != 3 {
            return Err(JsValue::from_str(
                "player bootstrap must contain three strings",
            ));
        }
        let player_string = |index: u32, name: &str| {
            player.get(index).as_string().ok_or_else(|| {
                JsValue::from_str(&format!("player bootstrap {name} is not a string"))
            })
        };
        let browser_user_id = player_string(0, "browser user id")?;
        let player_id = player_string(1, "player id")?;
        let player_name = player_string(2, "name")?;
        let identity = PlayerIdentity {
            browser_user_id: BrowserUserId::from_uuid_str(&browser_user_id)
                .ok_or_else(|| JsValue::from_str("browser user id is not a UUID"))?,
            player_id: PlayerId::from_uuid_str(&player_id)
                .ok_or_else(|| JsValue::from_str("player id is not a UUID"))?,
            player_name,
        };
        identity
            .validate()
            .map_err(|error| JsValue::from_str(&format!("player identity: {error}")))?;
        let client_config = ClientConfig::from_toml(&config_toml)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let client_config_hash = reproduction_config_hash(&client_config)
            .map_err(|error| JsValue::from_str(&format!("hash client configuration: {error}")))?;
        let developer_controls_enabled = client_config.developer.controls_enabled;
        let world_transport = client_config.world.clone();
        let runtime = client_config.runtime;
        let streaming = &client_config.streaming;
        let diagnostics = client_config.diagnostics;
        let profiling = client_config.profiling;
        let engine_config = EngineConfig {
            developer_controls_enabled,
            fixed_step_seconds: runtime.fixed_step_seconds,
            max_steps_per_frame: runtime.max_steps_per_frame,
            max_edit_trackers: runtime.max_edit_trackers as usize,
            stream_frame_budget: FrameBudget {
                generation: streaming.frame_budget.generation as usize,
                meshing: streaming.frame_budget.meshing as usize,
                upload: streaming.frame_budget.upload as usize,
            },
            startup_ready_radius_chunks: streaming.startup_ready_radius_chunks as i32,
            stream_collision_lookahead_seconds: streaming.priority.collision_lookahead_seconds,
            stream_velocity_lookahead_seconds: streaming.priority.velocity_lookahead_seconds,
            stream_view_cone_half_angle_degrees: streaming.priority.view_cone_half_angle_degrees,
            stream_enclosed_view_distance_metres: streaming.priority.enclosed_view_distance_metres,
            view_distance_metres: client_config.rendering.view_distance_metres,
            enclosure_probe_interval_ms: f64::from(diagnostics.enclosure_probe_interval_ms),
            enclosure_probe_distance_metres: diagnostics.enclosure_probe_distance_metres,
        };
        let rendering = &client_config.rendering;
        let mut renderer_config = RendererConfig {
            features: RendererFeatureConfig {
                cascaded_sun_shadows: rendering.features.cascaded_sun_shadows,
                voxel_ambient_occlusion: rendering.features.voxel_ambient_occlusion,
                screen_space_ambient_occlusion: rendering.features.screen_space_ambient_occlusion,
                atmospheric_fog: rendering.features.atmospheric_fog,
                far_terrain: rendering.features.far_terrain,
                water_surface: rendering.features.water_surface,
                target_outline: rendering.features.target_outline,
                material_surface_detail: rendering.features.material_surface_detail,
                cave_headlamp: rendering.features.cave_headlamp,
                voxel_emissive_lights: rendering.features.voxel_emissive_lights,
            },
            mission_control: MissionControlConfig {
                open: rendering.mission_control.open,
                developer_controls: developer_controls_enabled,
                spectator_available: false,
            },
            view_distance_metres: rendering.view_distance_metres,
            directional_shadows: DirectionalShadowConfig {
                vertical_fov_radians: rendering.shadows.vertical_fov_radians,
                near_plane: rendering.shadows.near_plane,
                far_plane: rendering.shadows.far_plane,
                split_lambda: rendering.shadows.split_lambda,
                shadow_map_resolution: rendering.shadows.shadow_map_resolution,
                direction_update_threshold_radians: rendering
                    .shadows
                    .direction_update_threshold_radians,
                caster_depth_expansion: rendering.shadows.caster_depth_expansion,
            },
            volumetric_clouds: VolumetricCloudConfig {
                enabled: rendering.volumetric_clouds.enabled,
                resolution_scale: rendering.volumetric_clouds.resolution_scale,
                view_steps: rendering.volumetric_clouds.view_steps,
                light_steps: rendering.volumetric_clouds.light_steps,
                max_distance_metres: rendering.volumetric_clouds.max_distance_metres,
                extinction: rendering.volumetric_clouds.extinction,
            },
            diagnostic_sky_color: rendering
                .diagnostics
                .sky_override_rgb
                .map(|color| color.map(|channel| f32::from(channel) / 255.0)),
        };
        let width = (css_width * dpr).round().max(1.0) as u32;
        let height = (css_height * dpr).round().max(1.0) as u32;
        let remote = RemoteWorldClient::connect(world_transport.clone(), identity.clone())
            .await
            .map_err(|error| JsValue::from_str(&format!("connect world service: {error}")))?;
        let opened = remote
            .world_opened()
            .ok_or_else(|| JsValue::from_str("world handshake completed without a manifest"))?;
        renderer_config.mission_control.spectator_available = developer_controls_enabled
            && opened
                .capabilities
                .contains(WorldCapabilities::SPECTATOR_MODE);
        let edits = EditMap::default();
        let spawn = opened.spawn;
        let resume = opened.player_resume;
        let mut camera = crate::camera_from_resume_values([
            resume.eye_position_metres[0],
            resume.eye_position_metres[1],
            resume.eye_position_metres[2],
            resume.look_yaw_radians,
            resume.look_pitch_radians,
        ]);
        let spectator = runtime.spectator;
        if !camera.set_spectator_flight_config(SpectatorFlightConfig {
            initial_speed_metres_per_second: spectator.initial_speed_metres_per_second,
            maximum_speed_metres_per_second: spectator.maximum_speed_metres_per_second,
            acceleration_metres_per_second_squared: spectator
                .acceleration_metres_per_second_squared,
            direction_response_per_second: spectator.direction_response_per_second,
            stopping_response_per_second: spectator.stopping_response_per_second,
        }) {
            return Err(JsValue::from_str(
                "validated spectator flight configuration was rejected by simulation",
            ));
        }
        let presence =
            RemotePresenceClient::start(world_transport, client_config.multiplayer, &opened)
                .map_err(|error| JsValue::from_str(&format!("connect player presence: {error}")))?;
        let renderer = Renderer::new(
            wgpu::SurfaceTarget::OffscreenCanvas(canvas),
            width,
            height,
            dpr,
            log_gpu_error,
            renderer_config,
        )
        .await
        .map_err(|error| JsValue::from_str(&error))?;
        let mut renderer = renderer;
        renderer.set_reduced_motion(reduced_motion);
        renderer.set_inventory_counts(opened.inventory.counts);
        renderer.set_screenshot_world_manifest(&opened.manifest);
        renderer.set_screenshot_reproduction_identity(ScreenshotReproductionIdentity {
            build_commit: option_env!("VOXELS_BUILD_COMMIT")
                .unwrap_or("unknown")
                .to_owned(),
            build_dirty: option_env!("VOXELS_BUILD_DIRTY") == Some("true"),
            build_profile: option_env!("VOXELS_BUILD_PROFILE")
                .unwrap_or("unknown")
                .to_owned(),
            protocol_version: voxels_world::protocol::PROTOCOL_VERSION,
            client_config_hash,
        });
        let scheduler = StreamScheduler::new(StreamConfig {
            load_radius_chunks: streaming.load_radius_chunks as i32,
            vertical_radius_chunks: streaming.vertical_radius_chunks as i32,
            retention_margin_chunks: streaming.retention_margin_chunks as i32,
            max_tracked_chunks: streaming.max_tracked_chunks as usize,
            max_secondary_interest_chunks: streaming.max_secondary_interest_chunks as usize,
        })
        .map_err(|error| JsValue::from_str(&format!("stream configuration: {error:?}")))?;
        let remote_environment = (
            AtmosphereSample {
                humidity: spawn.moisture,
                coldness: 1.0 - spawn.temperature,
                aerosol: spawn.ridge,
                cloudiness: (spawn.moisture + spawn.ridge) * 0.5,
                horizon_warmth: spawn.temperature,
                haze: spawn.moisture * 0.5,
            },
            spawn.region,
        );
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let engine = Rc::new(Engine {
            config: engine_config,
            renderer: RefCell::new(renderer),
            viewport_size: Cell::new([width, height]),
            camera: RefCell::new(camera),
            reproduction_camera: Cell::new(None),
            reproduction_restore_camera: Cell::new(None),
            spectator_body: Cell::new(None),
            input: RefCell::new(InputState::default()),
            remote,
            presence,
            environment_snapshot: Cell::new(opened.environment),
            source_identity_hash: opened.manifest.source_identity_hash(),
            remote_environment,
            edits: RefCell::new(edits),
            inventory: Cell::new(opened.inventory),
            edit_revisions: RefCell::new(AuthoritativeEditRevisions::default()),
            scheduler: RefCell::new(scheduler),
            chunks: RefCell::new(BTreeMap::new()),
            chunk_portals: RefCell::new(BTreeMap::new()),
            chunk_halos: RefCell::new(BTreeMap::new()),
            pending_meshes: RefCell::new(BTreeMap::new()),
            pending_uploads: RefCell::new(BTreeMap::new()),
            canonical_publications: RefCell::new(VecDeque::new()),
            binary_mesh_scratch: RefCell::new(BinaryMeshScratch::default()),
            virtual_terrain: RefCell::new(VirtualTerrainStreamingState::default()),
            virtual_terrain_scheduler: RefCell::new(
                TerrainStreamScheduler::new(TerrainStreamConfig::INTERACTIVE_CLIENT)
                    .map_err(|error| JsValue::from_str(&error.to_string()))?,
            ),
            virtual_terrain_cache: RefCell::new(
                TerrainPageMemoryCache::new(
                    opened.manifest.source_identity_hash(),
                    VIRTUAL_TERRAIN_PAGE_CACHE_BYTES,
                )
                .map_err(|error| JsValue::from_str(&error.to_string()))?,
            ),
            terrain_ready: Cell::new(false),
            startup_ready: Cell::new(false),
            scope,
            callback: RefCell::new(None),
            frame_id: Cell::new(0),
            frame_sequence: Cell::new(0),
            last_time: Cell::new(0.0),
            simulation_accumulator: Cell::new(0.0),
            frame_milliseconds: Cell::new(0.0),
            cpu_milliseconds: Cell::new(0.0),
            simulation_milliseconds: Cell::new(0.0),
            stream_milliseconds: Cell::new(0.0),
            render_milliseconds: Cell::new(0.0),
            frame_history: RefCell::new(FrameHistory::new()),
            edit_trackers: RefCell::new(VecDeque::new()),
            edit_last_ms: Cell::new(0.0),
            enclosure: Cell::new(EnclosureSample::OPEN),
            directional_light_occluded: Cell::new(false),
            last_enclosure_probe: Cell::new(f64::NEG_INFINITY),
            enclosure_probe_microseconds: Cell::new(0.0),
            cinder_portal_state: Cell::new(PortalState::default()),
            cinder_portal_revision: Cell::new(0),
            cinder_stream_interest: Cell::new(CaveStreamInterest::empty()),
            radial_active_chunks: RefCell::new(BTreeSet::new()),
            portal_active_chunks: RefCell::new(BTreeSet::new()),
            interaction_active_chunks: RefCell::new(BTreeSet::new()),
            enclosed_view_active_chunks: RefCell::new(BTreeSet::new()),
            enclosed_view_frontiers: RefCell::new(Vec::new()),
            surface_active_chunks: RefCell::new(BTreeSet::new()),
            touch_inventory_drag: Cell::new(None),
            profile: RefCell::new(ProfileAutomation::with_config(ProfileConfig {
                fixed_step_seconds: engine_config.fixed_step_seconds,
                speed_metres_per_second: profiling.speed_metres_per_second,
                warmup_seconds: profiling.warmup_seconds,
                measure_seconds: profiling.measure_seconds,
            })),
            profile_restore_camera: Cell::new(None),
            profile_tracked_high: Cell::new(0),
            profile_pending_high: Cell::new(0),
            profile_pending_mesh_high: Cell::new(0),
            profile_arena_capacity_high: Cell::new(0),
            profile_wasm_high: Cell::new(0),
            profile_start_evictions: Cell::new(0),
            stopped: Cell::new(false),
        });
        engine.start()?;
        Ok(EngineHandle {
            engine: Some(engine),
        })
    }

    const fn coord_key(coord: ChunkCoord) -> (i32, i32, i32) {
        (coord.x, coord.y, coord.z)
    }

    fn usize_to_u32(value: usize) -> u32 {
        u32::try_from(value).unwrap_or(u32::MAX)
    }

    fn smoothed_ms(previous: f32, sample: f32) -> f32 {
        if previous <= 0.0 {
            sample
        } else {
            previous * 0.9 + sample * 0.1
        }
    }

    fn performance_now(performance: Option<&web_sys::Performance>) -> f64 {
        performance.map_or(0.0, web_sys::Performance::now)
    }

    fn wasm_committed_bytes() -> u64 {
        let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().unchecked_into();
        let buffer: js_sys::ArrayBuffer = memory.buffer().unchecked_into();
        u64::from(buffer.byte_length())
    }

    fn world_to_chunk(position: glam::Vec3) -> ChunkCoord {
        let edge_metres = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
        ChunkCoord::new(
            (position.x / edge_metres).floor() as i32,
            (position.y / edge_metres).floor() as i32,
            (position.z / edge_metres).floor() as i32,
        )
    }

    fn directional_stream_priority(
        camera: &CameraState,
        streaming_velocity: glam::Vec3,
        cell_size_metres: f32,
        lookahead_seconds: f32,
        cone_half_angle_degrees: f32,
    ) -> DirectionalStreamPriority {
        let forward = camera.forward();
        DirectionalStreamPriority::from_motion(
            [forward.x, forward.z],
            [
                streaming_velocity.x / cell_size_metres,
                streaming_velocity.z / cell_size_metres,
            ],
            lookahead_seconds,
            cone_half_angle_degrees,
        )
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn portal_mask(open_faces: &[usize]) -> ChunkPortalMask {
        let mut faces = 0_u8;
        let mut face_cells = [[0_u64; CHUNK_FACE_WORDS]; 6];
        for face in open_faces {
            faces |= 1 << face;
            face_cells[*face].fill(u64::MAX);
        }
        ChunkPortalMask {
            voxel_components: vec![1; voxels_world::CHUNK_EDGE.pow(3)].into_boxed_slice(),
            component_faces: vec![0, faces],
            component_face_cells: vec![[[0; CHUNK_FACE_WORDS]; 6], face_cells],
        }
    }

    fn straight_tunnel_portals(
        min_z: i32,
        max_z: i32,
    ) -> BTreeMap<(i32, i32, i32), ChunkPortalMask> {
        (min_z..=max_z)
            .map(|z| ((0, 1, z), portal_mask(&[4, 5])))
            .collect()
    }

    fn test_view_cone_tangent() -> f32 {
        viewport_view_cone_tangent(55.0, CAMERA_VERTICAL_FOV_RADIANS, 1_920, 1_080).unwrap()
    }

    #[test]
    fn server_resume_values_are_sanitized_before_use() {
        let valid = [12.0, 4.5, -8.0, 0.75, -0.4];
        let camera = camera_from_resume_values(valid);
        assert_eq!(
            [
                camera.position.x,
                camera.position.y,
                camera.position.z,
                camera.yaw,
                camera.pitch,
            ],
            valid
        );

        for recovered in [
            [f32::NAN, 4.5, -8.0, 0.75, -0.4],
            [12.0, 4.5, -8.0, 1.0e30, -0.4],
            [12.0, 4.5, -8.0, 0.75, 4.0],
        ] {
            let camera = camera_from_resume_values(recovered);
            assert!(camera.position.is_finite());
            assert!(camera.yaw.is_finite());
            assert!(camera.pitch.is_finite());
        }
    }

    #[test]
    fn presence_heartbeat_expires_only_after_a_complete_timeout_window() {
        assert!(!presence_heartbeat_expired(10_249.0, 250.0, 10_000));
        assert!(presence_heartbeat_expired(10_250.0, 250.0, 10_000));
        assert!(!presence_heartbeat_expired(
            10_250.0,
            f64::NEG_INFINITY,
            10_000
        ));
        assert!(!presence_heartbeat_expired(f64::NAN, 250.0, 10_000));
    }

    #[test]
    fn urgent_interest_reaches_below_a_downward_edit_corridor() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        camera.pitch = -1.5;
        let interest = urgent_stream_interest(&camera, glam::Vec3::ZERO, 2.0);

        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 1, 0)));
        assert!(
            interest
                .iter()
                .any(|coord| coord.x == 0 && coord.y <= -1 && coord.z == 0)
        );
        assert!(interest.len() <= 36, "lookahead must stay tightly bounded");
        assert!(interest.iter().all(|coord| coord.is_world_representable()));
    }

    #[test]
    fn urgent_interest_is_stable_across_negative_chunk_boundaries() {
        let mut camera = CameraState::spawn(glam::Vec3::new(-3.21, 0.05, -3.21));
        camera.pitch = -1.5;
        let first = urgent_stream_interest(&camera, glam::Vec3::ZERO, 2.0);
        let second = urgent_stream_interest(&camera, glam::Vec3::ZERO, 2.0);

        assert_eq!(first, second);
        assert!(first.contains(&voxels_world::ChunkCoord::new(-2, 0, -2)));
        assert!(first.iter().any(|coord| coord.y < 0));
    }

    #[test]
    fn urgent_interest_covers_current_support_and_projected_glider_path() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let interest = urgent_stream_interest(&camera, glam::Vec3::new(8.3, -2.2, 0.0), 2.0);

        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 0, 0)));
        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 1, 0)));
        assert!(
            interest.iter().any(|coord| coord.x >= 5 && coord.z == 0),
            "two seconds of glider motion must be requested ahead"
        );
        assert!(
            interest.iter().any(|coord| coord.z <= -2),
            "the independent look/edit corridor must remain urgent"
        );
        assert!(
            interest.len() <= 64,
            "default urgency must stay tightly bounded"
        );
    }

    #[test]
    fn movement_readiness_excludes_the_independent_edit_ray() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let velocity = glam::Vec3::new(8.0, 0.0, 0.0);
        let movement = movement_stream_interest(camera.position, velocity, 1.5);
        let urgent = urgent_stream_interest(&camera, velocity, 2.0);

        assert!(movement.iter().all(|coord| coord.z == 0));
        assert!(movement.iter().any(|coord| coord.x >= 4));
        assert!(urgent.iter().any(|coord| coord.z < 0));
        assert!(movement.iter().all(|coord| urgent.contains(coord)));
    }

    #[test]
    fn fast_spectator_flight_does_not_create_a_collision_critical_chunk_trail() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 80.0, 1.6));
        let velocity = glam::Vec3::new(128.0, 20.0, -32.0);
        assert_eq!(exact_streaming_velocity(&camera, velocity), velocity);

        camera.set_locomotion(voxels_core::LocomotionMode::Spectator);
        assert_eq!(
            exact_streaming_velocity(&camera, velocity),
            glam::Vec3::ZERO
        );
        let interest = movement_stream_interest(
            camera.position,
            exact_streaming_velocity(&camera, velocity),
            2.5,
        );
        assert!(
            interest.len() <= 8,
            "bodyless flight must not spend exact-chunk urgency along its entire cruise path"
        );
        let visual_interest = movement_stream_interest(camera.position, velocity, 1.5);
        assert!(
            visual_interest.iter().any(|coord| coord.x >= 20),
            "ordinary visual interest must still lead a fast spectator by several round trips"
        );
        assert!(
            visual_interest.len() <= 192,
            "visual lookahead must fit the scheduler's hard secondary-interest bound"
        );

        let led = predictive_stream_position(camera.position, velocity, 1.5, 12.8);
        let horizontal_lead = glam::Vec2::new(led.x - camera.position.x, led.z - camera.position.z);
        assert!((horizontal_lead.length() - 12.8).abs() < 0.001);
        assert_eq!(led.y, camera.position.y);
        assert_eq!(
            predictive_stream_position(camera.position, glam::Vec3::ZERO, 1.5, 12.8),
            camera.position
        );
    }

    #[test]
    fn enclosed_view_interest_reaches_beyond_the_surface_handoff() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let portals = straight_tunnel_portals(-9, 0);
        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(
            interest.iter().any(|coord| coord.z <= -4),
            "the exact corridor must cross the 12.8m canonical-to-surface handoff"
        );
        assert!(
            interest.iter().any(|coord| coord.z <= -9),
            "the configured corridor must retain a terminator near 32m"
        );
        assert!(
            interest.len() <= 24,
            "a straight portal corridor must stay well below the secondary-interest budget"
        );
    }

    #[test]
    fn unknown_tunnel_frontier_is_opaque_until_its_exact_mesh_is_renderable() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let current = voxels_world::ChunkCoord::new(0, 1, 0);
        let neighbor = voxels_world::ChunkCoord::new(0, 1, -1);
        let portals = BTreeMap::from([((0, 1, 0), portal_mask(&[4, 5]))]);
        let plan = enclosed_view_stream_plan(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(plan.chunks.contains(&neighbor));
        let caps = exact_volume_frontier_faces(&plan.frontiers, |coord| coord == current);
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].chunk, current);
        assert_eq!(caps[0].face, 4);
        assert!(caps[0].cells.iter().any(|word| *word != 0));

        let settled = exact_volume_frontier_faces(&plan.frontiers, |coord| {
            coord == current || coord == neighbor
        });
        assert!(
            settled.is_empty(),
            "the conservative wall must disappear when the exact neighbor owns the opening"
        );
    }

    #[test]
    fn rotating_in_place_republishes_changed_portal_frontier_cells() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let portals = BTreeMap::from([((0, 1, 0), portal_mask(&[4]))]);
        let first = enclosed_view_stream_plan(&camera, 32.0, test_view_cone_tangent(), &portals);

        camera.yaw += 0.35;
        let turned = enclosed_view_stream_plan(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert_eq!(
            first.chunks, turned.chunks,
            "this regression requires unchanged scheduler interest"
        );
        assert_ne!(
            first.frontiers, turned.frontiers,
            "camera rotation must change the visible face-cell mask"
        );
        let mut published = first.frontiers;
        assert!(replace_portal_frontiers(&mut published, &turned.frontiers));
        assert_eq!(published, turned.frontiers);
        assert!(!replace_portal_frontiers(&mut published, &turned.frontiers));
    }

    #[test]
    fn enclosed_view_interest_owns_nearby_occluders_without_streaming_behind_them() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let portals = BTreeMap::from([
            ((0, 1, 0), portal_mask(&[4, 5])),
            ((0, 1, -1), portal_mask(&[])),
        ]);
        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(
            !interest.is_empty(),
            "a wall used to reject a view ray must remain an exact render owner"
        );
        assert!(
            interest.iter().all(|coord| coord.z >= -1),
            "a nearby wall must stop exact-volume interest from extending behind it"
        );
    }

    #[test]
    fn enclosed_view_interest_stays_complete_across_the_visible_heading_range() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let portals = straight_tunnel_portals(-6, 2);
        let first =
            enclosed_view_stream_interest(&camera, 24.0, test_view_cone_tangent(), &portals);

        camera.yaw += 0.55;
        let turned =
            enclosed_view_stream_interest(&camera, 24.0, test_view_cone_tangent(), &portals);

        for z in -6..=0 {
            let coord = voxels_world::ChunkCoord::new(0, 1, z);
            assert!(first.contains(&coord));
            assert!(
                turned.contains(&coord),
                "visible tunnel chunk {z} was revoked"
            );
        }
    }

    #[test]
    fn enclosed_view_cone_covers_ultrawide_viewport_corners() {
        let ultrawide =
            viewport_view_cone_tangent(55.0, CAMERA_VERTICAL_FOV_RADIANS, 3_440, 1_440).unwrap();
        let vertical = (CAMERA_VERTICAL_FOV_RADIANS * 0.5).tan();
        let horizontal = vertical * (3_440.0 / 1_440.0);
        let viewport_corner = vertical.hypot(horizontal);

        assert!(
            ultrawide >= viewport_corner,
            "the circular portal cone must enclose every viewport corner"
        );
        assert!(
            ultrawide > 55.0_f32.to_radians().tan(),
            "a fixed 55 degree cone clips an ultrawide viewport"
        );
        assert!(viewport_view_cone_tangent(55.0, CAMERA_VERTICAL_FOV_RADIANS, 0, 1_440).is_none());
    }

    #[test]
    fn tall_cavern_keeps_exact_view_interest_without_lighting_enclosure() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let mut portals = straight_tunnel_portals(-6, 0);
        portals.insert((0, 1, 0), portal_mask(&[3, 4, 5]));
        for y in 2..=5 {
            portals.insert((0, y, 0), portal_mask(&[2, 3]));
        }
        portals.insert((0, 6, 0), portal_mask(&[]));

        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(
            interest.contains(&voxels_world::ChunkCoord::new(0, 1, -6)),
            "a ceiling more than the 12m lighting-probe distance above the camera must not disable exact cavern geometry"
        );
    }

    #[test]
    fn broad_portal_to_open_sky_does_not_flood_the_view_cone() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let mut portals = BTreeMap::new();
        for y in 1..=11 {
            portals.insert((0, y, 0), portal_mask(&[2, 3, 4, 5]));
            portals.insert((0, y, -1), portal_mask(&[4, 5]));
        }

        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(
            interest.iter().all(|coord| coord.x == 0 && coord.z == 0),
            "an outdoor proof may retain its narrow probe column but must never flood the view cone"
        );
        assert!(interest.len() <= 11);
    }

    #[test]
    fn stepping_from_proven_sky_into_a_pending_sky_probe_does_not_publish_a_stone_cap() {
        let mut portals = BTreeMap::new();
        for y in 1..=11 {
            portals.insert((0, y, 0), portal_mask(&[2, 3, 4, 5]));
        }
        portals.insert((0, 1, -1), portal_mask(&[2, 3, 4, 5]));

        let proven_camera = CameraState::spawn(glam::Vec3::new(0.05, 3.25, 0.05));
        let proven =
            enclosed_view_stream_plan(&proven_camera, 32.0, test_view_cone_tangent(), &portals);
        assert!(proven.frontiers.is_empty());

        let stepped_camera = CameraState::spawn(glam::Vec3::new(0.05, 3.25, -0.01));
        let pending =
            enclosed_view_stream_plan(&stepped_camera, 32.0, test_view_cone_tangent(), &portals);
        assert!(
            pending
                .chunks
                .contains(&voxels_world::ChunkCoord::new(0, 2, -1)),
            "the sky probe must keep streaming after the chunk-boundary step"
        );
        assert!(
            pending.frontiers.is_empty(),
            "an unresolved broad sky probe must not become opaque Stone geometry"
        );
    }

    #[test]
    fn narrow_upward_shaft_keeps_its_bounded_frontier_cap() {
        let coord = voxels_world::ChunkCoord::new(0, 1, 0);
        let mut chunk = voxels_world::Chunk::filled(coord, voxels_world::Material::Stone);
        for y in 0..voxels_world::CHUNK_EDGE {
            chunk.set(16, y, 16, voxels_world::Material::Air);
        }
        let portals = BTreeMap::from([((coord.x, coord.y, coord.z), {
            ChunkPortalMask::from_chunk(&chunk)
        })]);
        let mut camera = CameraState::spawn(glam::Vec3::new(1.65, 3.25, 1.65));
        camera.pitch = std::f32::consts::FRAC_PI_2;

        let plan = enclosed_view_stream_plan(&camera, 32.0, 100.0, &portals);

        assert!(
            plan.frontiers
                .iter()
                .any(|frontier| frontier.source == coord && frontier.face == 3),
            "the fix must preserve conservative ceilings for genuinely bounded narrow shafts"
        );
    }

    #[test]
    fn enclosed_view_interest_does_not_follow_connected_air_behind_the_camera() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        camera.yaw = std::f32::consts::PI;
        let portals = straight_tunnel_portals(-9, 0);
        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(interest.iter().all(|coord| coord.z >= 0));
    }

    #[test]
    fn enclosed_view_interest_keeps_shallow_tunnels_separate_from_sky_in_the_same_chunk() {
        let split_portals = |coord| {
            let mut chunk = voxels_world::Chunk::filled(coord, voxels_world::Material::Stone);
            for z in 0..voxels_world::CHUNK_EDGE {
                chunk.set(16, 16, z, voxels_world::Material::Air);
            }
            for z in 0..voxels_world::CHUNK_EDGE {
                for x in 0..voxels_world::CHUNK_EDGE {
                    chunk.set(
                        x,
                        voxels_world::CHUNK_EDGE - 1,
                        z,
                        voxels_world::Material::Air,
                    );
                }
            }
            ChunkPortalMask::from_chunk(&chunk)
        };
        let current = split_portals(voxels_world::ChunkCoord::new(0, 0, 0));
        assert_ne!(
            current.component_at(16, 16, 0),
            current.component_at(16, voxels_world::CHUNK_EDGE - 1, 0),
            "the tunnel and outdoor layer must remain separate portal components"
        );
        let portals = BTreeMap::from([
            ((0, 0, 0), current),
            (
                (0, 0, -1),
                split_portals(voxels_world::ChunkCoord::new(0, 0, -1)),
            ),
            ((1, 0, -1), portal_mask(&[0, 1, 4, 5])),
        ]);
        let camera = CameraState::spawn(glam::Vec3::new(1.65, 1.65, 1.65));

        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 0, -1)));
        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 0, -2)));
        assert!(
            !interest.contains(&voxels_world::ChunkCoord::new(1, 0, -1)),
            "outdoor air sharing the chunk must not widen the tunnel component into the sky"
        );
    }

    #[test]
    fn enclosed_view_interest_does_not_turn_up_a_shaft_outside_the_view_cone() {
        let coord = voxels_world::ChunkCoord::new(0, 0, 0);
        let mut chunk = voxels_world::Chunk::filled(coord, voxels_world::Material::Stone);
        for z in 0..voxels_world::CHUNK_EDGE {
            chunk.set(16, 16, z, voxels_world::Material::Air);
        }
        for y in 16..voxels_world::CHUNK_EDGE {
            chunk.set(16, y, 16, voxels_world::Material::Air);
        }
        let portals = BTreeMap::from([
            ((0, 0, 0), ChunkPortalMask::from_chunk(&chunk)),
            ((0, 1, 0), portal_mask(&[2, 3])),
        ]);
        let camera = CameraState::spawn(glam::Vec3::new(1.65, 1.65, 1.65));

        let interest =
            enclosed_view_stream_interest(&camera, 32.0, test_view_cone_tangent(), &portals);

        assert!(interest.contains(&voxels_world::ChunkCoord::new(0, 0, -1)));
        assert!(
            !interest.contains(&voxels_world::ChunkCoord::new(0, 1, 0)),
            "a connected shaft above the camera must not redirect a forward tunnel view"
        );
    }

    #[test]
    fn chunk_portals_require_matching_air_cells() {
        let mut current = voxels_world::Chunk::filled(
            voxels_world::ChunkCoord::new(0, 0, 0),
            voxels_world::Material::Stone,
        );
        let mut neighbor = voxels_world::Chunk::filled(
            voxels_world::ChunkCoord::new(0, 0, -1),
            voxels_world::Material::Stone,
        );
        current.set(4, 7, 0, voxels_world::Material::Air);
        neighbor.set(
            5,
            7,
            voxels_world::CHUNK_EDGE - 1,
            voxels_world::Material::Air,
        );
        let current = ChunkPortalMask::from_chunk(&current);
        let mismatched = ChunkPortalMask::from_chunk(&neighbor);
        let tunnel_component = current.component_at(4, 7, 0);
        let visible = [u64::MAX; CHUNK_FACE_WORDS];
        assert!(
            current
                .connected_neighbor_components(tunnel_component, 4, &mismatched, &visible)
                .is_empty()
        );

        neighbor.set(
            4,
            7,
            voxels_world::CHUNK_EDGE - 1,
            voxels_world::Material::Air,
        );
        let matching = ChunkPortalMask::from_chunk(&neighbor);
        assert!(
            !current
                .connected_neighbor_components(tunnel_component, 4, &matching, &visible)
                .is_empty()
        );
    }

    #[test]
    fn incremental_portals_label_an_interior_tunnel_when_it_reaches_a_boundary() {
        let coord = voxels_world::ChunkCoord::new(0, 0, 0);
        let mut chunk = voxels_world::Chunk::filled(coord, voxels_world::Material::Stone);
        let mut mask = ChunkPortalMask::from_chunk(&chunk);
        let interior = (8..16).map(|z| [16, 16, z]).collect::<Vec<_>>();
        for &[x, y, z] in &interior {
            chunk.set(x, y, z, voxels_world::Material::Air);
        }
        mask.add_non_occluding_voxels(&chunk, &interior);
        assert_eq!(mask.component_at(16, 16, 8), 0);

        let opening = (0..8).map(|z| [16, 16, z]).collect::<Vec<_>>();
        for &[x, y, z] in &opening {
            chunk.set(x, y, z, voxels_world::Material::Air);
        }
        mask.add_non_occluding_voxels(&chunk, &opening);

        let component = mask.component_at(16, 16, 0);
        assert_ne!(component, 0);
        assert_eq!(mask.component_at(16, 16, 15), component);
        assert!(mask.component_opens_face(component, 4));
    }

    #[test]
    fn incremental_portals_merge_components_joined_by_digging() {
        let coord = voxels_world::ChunkCoord::new(0, 0, 0);
        let mut chunk = voxels_world::Chunk::filled(coord, voxels_world::Material::Stone);
        for x in 0..15 {
            chunk.set(x, 16, 16, voxels_world::Material::Air);
        }
        for x in 16..voxels_world::CHUNK_EDGE {
            chunk.set(x, 16, 16, voxels_world::Material::Air);
        }
        let mut mask = ChunkPortalMask::from_chunk(&chunk);
        assert_ne!(mask.component_at(0, 16, 16), mask.component_at(31, 16, 16));

        chunk.set(15, 16, 16, voxels_world::Material::Air);
        mask.add_non_occluding_voxels(&chunk, &[[15, 16, 16]]);

        let component = mask.component_at(0, 16, 16);
        assert_ne!(component, 0);
        assert_eq!(mask.component_at(31, 16, 16), component);
        assert!(mask.component_opens_face(component, 0));
        assert!(mask.component_opens_face(component, 1));
    }

    #[test]
    fn capacity_truncated_columns_do_not_replace_surface_cover() {
        let complete_column = [
            voxels_world::ChunkCoord::new(13, 4, -9),
            voxels_world::ChunkCoord::new(13, 5, -9),
            voxels_world::ChunkCoord::new(13, 6, -9),
        ];
        let truncated_column = [
            voxels_world::ChunkCoord::new(14, 4, -9),
            voxels_world::ChunkCoord::new(14, 5, -9),
            voxels_world::ChunkCoord::new(14, 6, -9),
        ];
        let interest = [complete_column, truncated_column].concat();
        let admitted = complete_column
            .into_iter()
            .chain(truncated_column[..2].iter().copied())
            .collect::<BTreeSet<_>>();

        let ready =
            complete_renderable_interest_columns(&interest, |coord| admitted.contains(&coord));

        assert!(
            complete_column
                .into_iter()
                .all(|coord| { ready.contains(&(coord.x, coord.y, coord.z)) })
        );
        assert!(
            truncated_column
                .into_iter()
                .all(|coord| { !ready.contains(&(coord.x, coord.y, coord.z)) })
        );
    }

    #[test]
    fn virtual_terrain_column_corridor_covers_every_crossed_deadline() {
        let columns = virtual_terrain_column_corridor([10, -4], [16, -1]);
        assert_eq!(columns.first(), Some(&[10, -4]));
        assert_eq!(columns.last(), Some(&[16, -1]));
        assert_eq!(columns.len(), 7);
        assert!(columns.windows(2).all(|pair| {
            let delta_x = (pair[1][0] - pair[0][0]).abs();
            let delta_z = (pair[1][1] - pair[0][1]).abs();
            delta_x <= 1 && delta_z <= 1 && delta_x + delta_z > 0
        }));
    }

    #[test]
    fn virtual_terrain_column_priority_does_not_revoke_completed_lookahead() {
        let prioritized = [[20, 0], [25, 0], [21, 0], [22, 0]];
        let completed = [[18, 0], [19, 0], [20, 0], [21, 0], [22, 0], [23, 0]];

        let keep = virtual_terrain_column_working_set(&prioritized, completed, 6);

        assert_eq!(
            keep,
            BTreeSet::from([[19, 0], [20, 0], [21, 0], [22, 0], [23, 0], [25, 0]])
        );
    }

    #[test]
    fn incomplete_candidate_cut_keeps_prior_registered_roots() {
        use voxels_world::{TERRAIN_COVERAGE_ROOT_LEVEL, TerrainPageKey};

        let root = |x| TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, x, -4);
        let registered = [root(8), root(9), root(10), root(11)];

        let keep = virtual_terrain_root_working_set(&[root(12)], registered, root(11), 4);

        assert_eq!(keep, BTreeSet::from(registered));
    }

    #[test]
    fn virtual_terrain_root_working_set_evicts_only_when_bounded() {
        use voxels_world::{TERRAIN_COVERAGE_ROOT_LEVEL, TerrainPageKey};

        let root = |x| TerrainPageKey::surface(TERRAIN_COVERAGE_ROOT_LEVEL, x, 7);
        let registered = [root(0), root(1), root(2), root(3), root(4)];

        let keep = virtual_terrain_root_working_set(&[root(4)], registered, root(4), 3);

        assert_eq!(keep, BTreeSet::from([root(2), root(3), root(4)]));
    }

    #[test]
    fn edit_revision_floors_follow_only_affected_surface_ancestor_chains() {
        use voxels_world::{ChunkCoord, TERRAIN_COVERAGE_ROOT_LEVEL, TerrainPageKey};

        let affected = [ChunkCoord::new(13, 4, -7), ChunkCoord::new(1_024, 4, -7)];
        let (roots, revision_keys) = virtual_terrain_edit_revision_keys(&affected);
        let first_leaf = TerrainPageKey::surface(0, 13, -7);
        let second_leaf = TerrainPageKey::surface(0, 1_024, -7);

        assert_eq!(
            roots,
            BTreeSet::from([
                first_leaf.ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL).unwrap(),
                second_leaf
                    .ancestor_at(TERRAIN_COVERAGE_ROOT_LEVEL)
                    .unwrap(),
            ])
        );
        assert!(
            (1..=TERRAIN_COVERAGE_ROOT_LEVEL)
                .all(|level| revision_keys.contains(&first_leaf.ancestor_at(level).unwrap()))
        );
        assert!(
            (1..=TERRAIN_COVERAGE_ROOT_LEVEL)
                .all(|level| revision_keys.contains(&second_leaf.ancestor_at(level).unwrap()))
        );
        assert!(
            !revision_keys.contains(&TerrainPageKey::surface(1, 7, -4)),
            "an unedited sibling must keep its own older spatial revision"
        );
    }

    #[test]
    fn pending_portal_frontier_does_not_revoke_ready_tunnel_sibling() {
        let visible_wall = voxels_world::ChunkCoord::new(69, 42, 137);
        let pending_frontier = voxels_world::ChunkCoord::new(69, 43, 137);
        let interest = [visible_wall, pending_frontier];
        let renderable = BTreeSet::from([visible_wall]);

        let active =
            renderable_exact_interest_chunks(&interest, |coord| renderable.contains(&coord));

        assert_eq!(active, BTreeSet::from([(69, 42, 137)]));
        assert!(
            complete_renderable_interest_columns(&interest, |coord| renderable.contains(&coord))
                .is_empty(),
            "the surface-column rule reproduces the tunnel flicker when one Y sibling is pending"
        );
    }

    #[test]
    fn horizontal_inventory_swipes_are_thresholded_and_directional() {
        assert_eq!(inventory_swipe([100.0, 500.0], [125.0, 503.0]), None);
        assert_eq!(
            inventory_swipe([100.0, 500.0], [66.0, 504.0]),
            Some((1, [66.0, 504.0]))
        );
        assert_eq!(
            inventory_swipe([100.0, 500.0], [170.0, 497.0]),
            Some((-2, [168.0, 497.0]))
        );
    }

    #[test]
    fn vertical_or_invalid_touch_motion_does_not_turn_the_inventory() {
        assert_eq!(inventory_swipe([100.0, 500.0], [140.0, 560.0]), None);
        assert_eq!(inventory_swipe([f32::NAN, 500.0], [140.0, 500.0]), None);
    }

    #[test]
    fn synchronized_clients_derive_identical_world_time_and_cloud_offset() {
        let snapshot = voxels_world::protocol::WorldEnvironmentSnapshot {
            sample_server_time_ms: 5_000,
            world_day_number: 82,
            day_fraction: 0.25,
            day_length_seconds: 100.0,
            days_per_year: 365.242_2,
            moon_sidereal_orbit_days: 27.321_661,
            moon_orbit_phase_at_world_epoch: 0.17,
            planet_circumference_metres: 40_075_016.0,
            axial_tilt_radians: 23.439_3_f32.to_radians(),
            moon_orbit_inclination_radians: 5.145_f32.to_radians(),
            celestial_seed: 0x57a2_5eed,
            celestial_revision: 2,
            weather_fraction: 0.1,
            weather_cycle_seconds: 200.0,
            cloud_offset_metres: [10.0, 20.0],
            cloud_velocity_metres_per_second: [4.0, -2.0],
            cloud_coverage: 0.6,
            cloud_base_metres: 420.0,
            cloud_top_metres: 780.0,
            weather_seed: 7,
            weather_revision: 3,
        };
        let first = world_environment_at(snapshot, 30_000.0);
        let second = world_environment_at(snapshot, 30_000.0);
        assert_eq!(first, second);
        assert_eq!(first.server_time_seconds, 30.0);
        assert!((first.day_fraction - 0.5).abs() < 1.0e-6);
        assert!((first.world_days - 82.5).abs() < 1.0e-9);
        assert!(
            (first.year_fraction - (82.5_f64 / 365.242_2).rem_euclid(1.0) as f32).abs() < 1.0e-6
        );
        assert!(
            (first.moon_orbit_fraction - (82.5_f64 / 27.321_661 + 0.17).rem_euclid(1.0) as f32)
                .abs()
                < 1.0e-6
        );
        assert!((first.twinkle_phase - (82.5_f64 * 37.0).rem_euclid(1.0) as f32).abs() < 1.0e-6);
        assert_eq!(first.celestial_seed, 0x57a2_5eed);
        assert_eq!(first.celestial_revision, 2);
        assert!((first.weather_fraction - 0.225).abs() < 1.0e-6);
        assert_eq!(first.cloud_offset_metres, [110.0, 1_279_970.0]);
    }

    #[test]
    fn atmosphere_motion_clock_retains_subframe_precision_at_unix_scale() {
        let snapshot = voxels_world::protocol::WorldEnvironmentSnapshot {
            sample_server_time_ms: 1_784_500_000_000,
            world_day_number: 0,
            day_fraction: 0.5,
            day_length_seconds: 0.0,
            days_per_year: 365.242_2,
            moon_sidereal_orbit_days: 27.321_661,
            moon_orbit_phase_at_world_epoch: 0.17,
            planet_circumference_metres: 40_075_016.0,
            axial_tilt_radians: 23.439_3_f32.to_radians(),
            moon_orbit_inclination_radians: 5.145_f32.to_radians(),
            celestial_seed: 1,
            celestial_revision: 1,
            weather_fraction: 0.5,
            weather_cycle_seconds: 0.0,
            cloud_offset_metres: [0.0; 2],
            cloud_velocity_metres_per_second: [0.0; 2],
            cloud_coverage: 0.5,
            cloud_base_metres: 550.0,
            cloud_top_metres: 1_800.0,
            weather_seed: 1,
            weather_revision: 1,
        };
        let first = world_environment_at(snapshot, 1_784_500_000_000.0);
        let next = world_environment_at(snapshot, 1_784_500_000_008.0);
        let elapsed = next.server_time_seconds - first.server_time_seconds;
        assert!((elapsed - 0.008).abs() < 0.0002, "{elapsed}");
    }

    #[test]
    fn hidden_tab_time_jump_catches_up_without_frame_delta_accumulation() {
        let snapshot = voxels_world::protocol::WorldEnvironmentSnapshot {
            sample_server_time_ms: 1_000,
            world_day_number: 9,
            day_fraction: 0.9,
            day_length_seconds: 40.0,
            days_per_year: 365.242_2,
            moon_sidereal_orbit_days: 27.321_661,
            moon_orbit_phase_at_world_epoch: 0.17,
            planet_circumference_metres: 40_075_016.0,
            axial_tilt_radians: 23.439_3_f32.to_radians(),
            moon_orbit_inclination_radians: 5.145_f32.to_radians(),
            celestial_seed: 0x57a2_5eed,
            celestial_revision: 2,
            weather_fraction: 0.68,
            weather_cycle_seconds: 0.0,
            cloud_offset_metres: [0.0, 0.0],
            cloud_velocity_metres_per_second: [5.0, 2.0],
            cloud_coverage: 0.4,
            cloud_base_metres: 420.0,
            cloud_top_metres: 780.0,
            weather_seed: 1,
            weather_revision: 1,
        };
        let resumed = world_environment_at(snapshot, 11_000.0);
        assert_eq!(resumed.server_time_seconds, 11.0);
        assert!((resumed.day_fraction - 0.15).abs() < 1.0e-6);
        assert!((resumed.world_days - 10.15).abs() < 1.0e-6);
        assert_eq!(resumed.weather_fraction, 0.68);
        assert_eq!(resumed.cloud_offset_metres, [50.0, 20.0]);
    }
}
