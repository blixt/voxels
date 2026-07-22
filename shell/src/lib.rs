//! Browser/WASM leaf for Voxels. The worker owns the renderer, clock, and input semantics.

#[cfg(any(target_arch = "wasm32", test))]
use voxels_core::CameraState;

#[cfg(any(target_arch = "wasm32", test))]
const INTERACTION_REACH_METRES: f32 = 5.0;
#[cfg(any(target_arch = "wasm32", test))]
const INTERACTION_STREAM_MARGIN_METRES: f32 = 0.7;
#[cfg(any(target_arch = "wasm32", test))]
const CHUNK_FACE_WORDS: usize = voxels_world::CHUNK_EDGE * voxels_world::CHUNK_EDGE / 64;
#[cfg(target_arch = "wasm32")]
const COLLISION_READINESS_RESERVE_SECONDS: f32 = 1.0;
#[cfg(any(target_arch = "wasm32", test))]
const INVENTORY_SWIPE_THRESHOLD_CSS_PIXELS: f32 = 34.0;

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
struct ChunkPortalMask {
    voxel_components: Box<[u16]>,
    component_faces: Vec<u8>,
    component_face_cells: Vec<[[u64; CHUNK_FACE_WORDS]; 6]>,
}

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
                let component = u16::try_from(component_faces.len())
                    .expect("a 32-cubed chunk has fewer than u16::MAX components");
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

    fn component_has_visible_face_cell(
        &self,
        component: u16,
        face: usize,
        visible_cells: &[u64; CHUNK_FACE_WORDS],
    ) -> bool {
        self.component_face_cells
            .get(usize::from(component))
            .is_some_and(|faces| {
                faces[face]
                    .iter()
                    .zip(visible_cells)
                    .any(|(component, visible)| component & visible != 0)
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
            let component = neighboring_components.first().copied().unwrap_or_else(|| {
                let component = u16::try_from(self.component_faces.len())
                    .expect("a 32-cubed chunk has fewer than u16::MAX components");
                self.component_faces.push(0);
                self.component_face_cells.push([[0; CHUNK_FACE_WORDS]; 6]);
                component
            });
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
        if axial < -cell_radius || camera_to_center.length() > distance_metres + cell_radius {
            continue;
        }
        let perpendicular_squared = (camera_to_center.length_squared() - axial * axial).max(0.0);
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
fn enclosed_view_stream_interest(
    camera: &CameraState,
    distance_metres: f32,
    cone_half_angle_degrees: f32,
    portals: &std::collections::BTreeMap<(i32, i32, i32), ChunkPortalMask>,
) -> Vec<voxels_world::ChunkCoord> {
    use std::collections::{BTreeSet, VecDeque};
    use voxels_world::{CHUNK_EDGE, VOXEL_SIZE_METRES};

    if !distance_metres.is_finite()
        || distance_metres <= 0.0
        || !cone_half_angle_degrees.is_finite()
        || !(0.0..90.0).contains(&cone_half_angle_degrees)
    {
        return Vec::new();
    }
    let chunk_size = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;
    let chunk_radius = chunk_size * 3.0_f32.sqrt() * 0.5;
    let cone_tangent = cone_half_angle_degrees.to_radians().tan();
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
    let camera_voxel = (camera.position / VOXEL_SIZE_METRES).floor().as_ivec3();
    if let Some(origin_portals) = portals.get(&(origin.x, origin.y, origin.z)) {
        let component = origin_portals.component_at(
            camera_voxel.x.rem_euclid(CHUNK_EDGE as i32) as usize,
            camera_voxel.y.rem_euclid(CHUNK_EDGE as i32) as usize,
            camera_voxel.z.rem_euclid(CHUNK_EDGE as i32) as usize,
        );
        if component != 0 {
            visited.insert((origin, component));
            queue.push_back((origin, component));
        }
    }
    chunks.insert(origin);
    while let Some((current, current_component)) = queue.pop_front() {
        chunks.insert(current);
        let Some(current_portals) = portals.get(&(current.x, current.y, current.z)) else {
            continue;
        };
        for ([dx, dy, dz], face) in directions {
            let visible_cells =
                visible_portal_cells(camera, current, face, distance_metres, cone_tangent);
            if !current_portals.component_opens_face(current_component, face)
                || !current_portals.component_has_visible_face_cell(
                    current_component,
                    face,
                    &visible_cells,
                )
            {
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
            let Some(neighbor_portals) = portals.get(&(x, y, z)) else {
                continue;
            };
            for neighbor_component in current_portals.connected_neighbor_components(
                current_component,
                face,
                neighbor_portals,
                &visible_cells,
            ) {
                if visited.insert((neighbor, neighbor_component)) {
                    queue.push_back((neighbor, neighbor_component));
                }
            }
        }
    }
    chunks.into_iter().collect()
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

#[cfg(any(target_arch = "wasm32", test))]
fn camera_from_resume_values(values: [f32; 5]) -> CameraState {
    CameraState::from_persisted(
        glam::Vec3::new(values[0], values[1], values[2]),
        values[3],
        values[4],
    )
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
        RemoteChunkCompletion, RemoteEditEvent, RemoteSurfaceCompletion, RemoteSurfaceTicket,
        RemoteWorldClient, RemoteWorldError,
    };
    use crate::{ChunkPortalMask, world_environment_at};
    use bytemuck::{Pod, Zeroable};
    use glam::{Vec2, Vec3};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use voxels_client_config::ClientConfig;
    use voxels_core::{
        CameraState, EnclosureSample, InputState, LocomotionMode, PLAYER_EYE_HEIGHT_METRES,
        PLAYER_HEIGHT_METRES, PLAYER_RADIUS_METRES, PLAYER_SPRINT_SPEED_METRES_PER_SECOND,
        ProfileAutomation, ProfileConfig, ProfilePhase, ProfileRoute, VoxelHit, VoxelPhysics,
        probe_enclosure, raycast_voxels, voxel_segment_is_clear,
    };
    use voxels_render::renderer::{
        ChunkActivationReason, HostUiAction, LocalLightVisibility, MissionControlConfig, Renderer,
        RendererConfig, RendererFeatureConfig, ScreenshotCapture, VolumetricCloudConfig,
    };
    use voxels_render::shadow::DirectionalShadowConfig;
    use voxels_render::ui::{LiveStats, NavigationTelemetry};
    use voxels_runtime::{
        AuthoritativeEditRevisions, ChunkState, CompletionStatus, DirectionalStreamPriority,
        FrameBudget, StreamConfig, StreamScheduler, SurfaceFocusAction, SurfaceRevisionCache,
        revision_satisfies,
    };
    use voxels_world::protocol::{
        BrowserUserId, EDIT_CUBE_EDGE_VOXELS, EDIT_CUBE_VOLUME_VOXELS, EDIT_SPHERE_RADIUS_VOXELS,
        EDIT_SPHERE_VOLUME_VOXELS, EditAction, EditShape, EditVolume, MaterialInventory, PlayerId,
        PlayerIdentity, VoxelMutation, WorldCapabilities, WorldEnvironmentSnapshot,
    };
    use voxels_world::{
        AtmosphereSample, BinaryMeshScratch, CHUNK_EDGE, CHUNK_VOXEL_BYTES,
        CINDER_VAULT_PORTAL_COUNT, CaveStreamInterest, Chunk, ChunkCoord, EditMap, Material,
        MeshedChunk, MeshingHalo, PortalState, SURFACE_LOD_LEVEL_COUNT, SurfaceLodLevel,
        SurfaceRegion, SurfaceSample, SurfaceTileCoord, VOXEL_SIZE_METRES, VoxelCoord,
        WorldProductPriority, WorldSourceIdentityHash, mesh_chunk_binary_with_scratch,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, OffscreenCanvas};

    const FRAME_HISTORY_CAPACITY: usize = 512;
    const AUTOMATION_CONTRACT_VERSION: u32 = 3;
    const SNAPSHOT_SCHEMA_VERSION: u32 = 37;
    const FRAME_SAMPLE_WIDTH: u32 = 14;
    const GPU_SAMPLE_WIDTH: u32 = 13;
    const SNAPSHOT_FIELD_NAMES: &str = concat!(
        "cameraX,cameraY,cameraZ,yaw,pitch,grounded,quads,edits,residentChunks,trackedChunks,visibleChunks,drawCalls,",
        "arenaPages,arenaAllocatedMiB,arenaCapacityMiB,pendingJobs,surfaceTiles,frameMs,shadowDrawCalls,shadowCascades,loadP95Frames,loadMaxFrames,remeshP95Frames,remeshMaxFrames,",
        "stride2Tiles,stride4Tiles,stride8Tiles,stride16Tiles,waterQuads,waterDrawCalls,refractionCopyMiB,immersion,eyeDepthMetres,eyesSubmerged,swimming,targetVoxelX,",
        "targetVoxelY,targetVoxelZ,targetPresent,coreGpuMiB,cpuMs,simulationMs,streamMs,renderMs,gpuSampleId,gpuTotalMs,gpuShadowMs,gpuWorldMs,",
        "gpuWaterMs,gpuUiMs,wasmCommittedMiB,canonicalVoxelMiB,pendingMeshMiB,editLogicalMiB,totalEvictions,staleCompletions,profilePhase,profileElapsedSeconds,profileDistanceMetres,profileComplete,",
        "profileTrackedHigh,profileSurfaceHigh,profilePendingHigh,profilePendingMeshHigh,profileArenaCapacityHighMiB,profileWasmHighMiB,profileEvictions,materialDetail,daylightPhase,surfaceRegion,cloudCoverage,screenSpaceAmbientOcclusion,",
        "gpuDepthPrepassMs,gpuAmbientOcclusionMs,ambientOcclusionMiB,depthPrepassDrawCalls,enclosure,interiorExposure,caveHeadlamp,enclosureProbeUs,localLightCandidates,activeLocalLights,clippedLocalLights,occludedLocalLights,",
        "portalRejectedLocalLights,localLightVisibilityTests,openCinderPortals,cinderPortalRevision,localLighting,placementMaterial,streamInterestRequested,streamInterestNormalized,streamInterestDesired,streamInterestTruncated,streamPlanOverflow,portalActiveChunks,",
        "portalActiveColumns,unreachablePortalActive,remoteAvatars,avatarParts,avatarDrawCalls,viewportFingerprintLow24,viewportFingerprintHigh24,allLodsReady,surfaceInFlight,interactiveLodsReady,stride32Tiles,stride64Tiles,stride128Tiles,stride256Tiles,",
        "renderCullMs,renderEncodeMs,renderSubmitMs,drawListTestedSlices,drawListSelectedSlices,surfaceWidth,surfaceHeight,devicePixelRatio,lodTransitionQuads,lodBoundary0X,lodBoundary0Z,lodBoundary1X,",
        "lodBoundary1Z,lodBoundary2X,lodBoundary2Z,lodBoundary3X,lodBoundary3Z,lodBoundary4X,lodBoundary4Z,lodBoundary5X,lodBoundary5Z,lodBoundary6X,lodBoundary6Z,lodBoundary7X,lodBoundary7Z,dayFraction,localSolarDayFraction,yearFraction,",
        "moonOrbitFraction,twinklePhase,latitudeDegrees,longitudeDegrees,localSiderealAngleRadians,moonIlluminatedFraction,celestialRevision,sunDirectionX,sunDirectionY,sunDirectionZ,moonDirectionX,moonDirectionY,",
        "moonDirectionZ,shadowStrength,cloudOffsetX,cloudOffsetZ,cloudVelocityX,cloudVelocityZ,weatherRevision,weatherKind,weatherFraction,precipitation,storminess,lightning,",
        "cloudDensity,cloudBaseMetres,cloudTopMetres,cloudRenderWidth,cloudRenderHeight,cloudViewSteps,cloudLightSteps,fogDensity,outdoorExposure,spectatorActive,presentedLodStrideVoxels,lodFocusLagVoxels,canonicalImmediateResident,canonicalImmediateRequired,canonicalSurfaceCellsResident,canonicalSurfaceCellsRequired,",
        "generationQueued,generationInFlight,meshingQueued,meshingInFlight,uploadQueued,uploadInFlight,surfaceQueued,surfaceDirty,loadCompleted,loadInFlight,acceptedCompletions,collisionImmediateResident,collisionImmediateRequired,collisionLookaheadResident,collisionLookaheadRequired,collisionLookaheadSeconds,editCanonicalRequired,editCanonicalRenderable,editCanonicalOwned,enclosedViewResident,enclosedViewRequired,enclosedViewRenderable,enclosedViewOwned,schemaVersion,sampleCount,",
        "droppedSamples",
    );
    const INTERACTIVE_SURFACE_LOD_LEVELS: usize = 4;
    const SURFACE_HINT_VERTICAL_MARGIN_CHUNKS: i32 = 1;
    const INTERACTIVE_SURFACE_BATCH: usize = 4;
    const BACKGROUND_SURFACE_BATCH: usize = 2;
    const BACKGROUND_SURFACE_BATCHES_IN_FLIGHT: usize = 4;
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
        stream_enclosed_view_threshold: f32,
        surface_load_radius_tiles: [i32; SURFACE_LOD_LEVEL_COUNT],
        surface_retain_margin_tiles: i32,
        enclosure_probe_interval_ms: f64,
        enclosure_probe_distance_metres: f32,
    }

    type FrameCallback = Closure<dyn FnMut(f64)>;
    type SurfaceChunkColumnHints = BTreeMap<(i32, i32), BTreeSet<i32>>;
    type SurfaceChunkHintIndex = BTreeMap<SurfaceTileCoord, SurfaceChunkColumnHints>;

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
        render_lod_plan_ms: f32,
        lod_plan_rebuild_reason: u32,
        render_encode_ms: f32,
        render_submit_ms: f32,
        lod_ownership_refreshes: u32,
        tested_slices: u32,
        selected_slices: u32,
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
                    sample.render_lod_plan_ms,
                    sample.lod_plan_rebuild_reason as f32,
                    sample.render_encode_ms,
                    sample.render_submit_ms,
                    sample.lod_ownership_refreshes as f32,
                    sample.tested_slices as f32,
                    sample.selected_slices as f32,
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

    #[derive(Clone, Copy, Debug)]
    struct SurfaceRequirement {
        coord: SurfaceTileCoord,
        revision: u64,
    }

    #[derive(Default)]
    struct EditRequirements {
        canonical: Vec<CanonicalRequirement>,
        surface: Vec<SurfaceRequirement>,
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

    struct Engine {
        config: EngineConfig,
        renderer: RefCell<Renderer>,
        camera: RefCell<CameraState>,
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
        surface_focus: Cell<Option<[SurfaceTileCoord; SURFACE_LOD_LEVEL_COUNT]>>,
        surface_resident: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_chunk_hints: RefCell<SurfaceChunkHintIndex>,
        surface_revisions: RefCell<SurfaceRevisionCache>,
        surface_accepted_edit_revisions: RefCell<BTreeMap<SurfaceTileCoord, u64>>,
        surface_queue: RefCell<VecDeque<SurfaceTileCoord>>,
        surface_in_flight: RefCell<BTreeSet<SurfaceTileCoord>>,
        surface_dirty: RefCell<BTreeSet<SurfaceTileCoord>>,
        all_lods_ready: Cell<bool>,
        interactive_lods_ready: Cell<bool>,
        full_lods_initialized: Cell<bool>,
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
        surface_active_chunks: RefCell<BTreeSet<(i32, i32, i32)>>,
        touch_inventory_drag: Cell<Option<[f32; 2]>>,
        profile: RefCell<ProfileAutomation>,
        profile_restore_camera: Cell<Option<CameraState>>,
        profile_tracked_high: Cell<usize>,
        profile_surface_high: Cell<usize>,
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
            self.profile_surface_high.set(0);
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
            let profiling = self.profile.borrow().running();
            let chunks = self.chunks.borrow();
            let mut accumulator = (self.simulation_accumulator.get() + dt.min(0.1))
                .min(self.config.fixed_step_seconds * self.config.max_steps_per_frame as f32);
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
            let streaming_velocity = if profiling {
                camera.velocity
            } else {
                camera.streaming_velocity(&self.input.borrow())
            };
            self.stream_world(&camera, streaming_velocity);
            if let Some(opened) = self.remote.world_opened() {
                self.presence.ensure_session(&opened, time);
                self.environment_snapshot.set(opened.environment);
            }
            // The streaming profiler moves a synthetic camera outside gameplay simulation. It may
            // stream/render that route, but it must never update authoritative player position or
            // gain edit reach from benchmark-only motion.
            let remote_avatars = self.presence.update(&camera, time, dt, !profiling);
            if let Some(error) = self.presence.take_error() {
                log_gpu_error(&format!("player presence: {error}"));
            }
            let stream_ms = (performance_now(performance.as_ref()) - stream_start) as f32;
            self.stream_milliseconds
                .set(smoothed_ms(self.stream_milliseconds.get(), stream_ms));
            let target = if camera.locomotion() == LocomotionMode::Spectator {
                None
            } else {
                let shape = self.renderer.borrow().edit_shape();
                self.dig_target(&camera, shape)
            };
            let mut renderer = self.renderer.borrow_mut();
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
            let lod_tiles = self.surface_lod_counts();
            let fine_coverage_ready = stream.generation.queued == 0
                && stream.generation.in_flight == 0
                && stream.meshing.queued == 0
                && stream.meshing.in_flight == 0
                && stream.upload.queued == 0
                && stream.upload.in_flight == 0;
            // Coverage controls startup readiness, not runtime geometric ownership. After the
            // hierarchy has initialized, its boundaries follow the camera continuously and each
            // missing child independently keeps its best resident parent. Holding an older,
            // whole-ring focus until every replacement settles cannot converge while moving.
            let ready_surface_levels = self.ready_surface_level_prefix();
            let interactive_lods_ready =
                fine_coverage_ready && ready_surface_levels >= INTERACTIVE_SURFACE_LOD_LEVELS;
            let all_lods_ready =
                fine_coverage_ready && ready_surface_levels == SURFACE_LOD_LEVEL_COUNT;
            self.all_lods_ready.set(all_lods_ready);
            self.interactive_lods_ready.set(interactive_lods_ready);
            debug_assert!(
                !all_lods_ready || self.surface_coverage_current(),
                "surface coverage became ready with missing or stale revisions"
            );
            let voxel_x = (camera.position.x / VOXEL_SIZE_METRES).floor() as i32;
            let voxel_z = (camera.position.z / VOXEL_SIZE_METRES).floor() as i32;
            if self.full_lods_initialized.get() {
                renderer.advance_geometric_lod_focus(
                    voxel_x,
                    voxel_z,
                    SURFACE_LOD_LEVEL_COUNT,
                    SURFACE_LOD_LEVEL_COUNT,
                );
            } else if ready_surface_levels > 0 {
                renderer.advance_geometric_lod_focus(
                    voxel_x,
                    voxel_z,
                    ready_surface_levels,
                    if all_lods_ready {
                        SURFACE_LOD_LEVEL_COUNT
                    } else {
                        ready_surface_levels
                    },
                );
                if all_lods_ready {
                    self.full_lods_initialized.set(true);
                }
            }
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
                        horizontal_speed_metres_per_second: camera
                            .velocity
                            .x
                            .hypot(camera.velocity.z),
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
                    resident_chunks: usize_to_u32(
                        stream.resident + self.surface_resident.borrow().len(),
                    ),
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
                    lod_tiles,
                    pending_jobs: usize_to_u32(
                        stream.generation.queued
                            + stream.meshing.queued
                            + stream.upload.queued
                            + self.surface_queue.borrow().len()
                            + self.surface_dirty.borrow().len(),
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
                    + stream.upload.in_flight
                    + self.surface_queue.borrow().len()
                    + self.surface_in_flight.borrow().len()
                    + self.surface_dirty.borrow().len();
                self.profile_tracked_high
                    .set(self.profile_tracked_high.get().max(stream.tracked));
                self.profile_surface_high.set(
                    self.profile_surface_high
                        .get()
                        .max(self.surface_resident.borrow().len()),
                );
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
                    && all_lods_ready
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
                render_lod_plan_ms: rendered.cpu_lod_plan_ms,
                lod_plan_rebuild_reason: rendered.lod_plan_rebuild_reason,
                render_encode_ms: rendered.cpu_encode_ms,
                render_submit_ms: rendered.cpu_submit_ms,
                lod_ownership_refreshes: rendered.lod_ownership_refreshes,
                tested_slices: rendered.draw_list_tested_slices,
                selected_slices: rendered.draw_list_selected_slices,
            });
            if let Err(error) = self.request_frame() {
                web_sys::console::error_1(&error);
                self.stopped.set(true);
            }
        }

        fn stream_world(&self, camera: &CameraState, streaming_velocity: Vec3) {
            self.drain_remote_generation();
            let focus = world_to_chunk(camera.position);
            let collision_interest = self.collision_stream_interest(
                camera,
                streaming_velocity,
                self.config.stream_collision_lookahead_seconds,
            );
            let enclosed_view_interest = self.enclosed_view_stream_interest(camera);
            let surface_interest = self.surface_stream_interest(focus, &collision_interest);
            let mut urgent_interest = collision_interest.clone();
            urgent_interest.extend(enclosed_view_interest.iter().copied());
            let mut interest = urgent_interest.clone();
            interest.extend(surface_interest.iter().copied());
            let priority_hint = directional_stream_priority(
                camera,
                streaming_velocity,
                CHUNK_EDGE as f32 * VOXEL_SIZE_METRES,
                self.config.stream_velocity_lookahead_seconds,
                self.config.stream_view_cone_half_angle_degrees,
            );
            let (focus_changed, work) = {
                let mut scheduler = self.scheduler.borrow_mut();
                let changed = scheduler.update_focus_with_interest(focus, &interest);
                (
                    changed,
                    scheduler.schedule_frame_prioritized_with_urgency(
                        self.config.stream_frame_budget,
                        priority_hint,
                        &urgent_interest,
                    ),
                )
            };
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
            if focus_changed || uploaded || evicted {
                self.reconcile_chunk_activation(
                    focus,
                    &collision_interest,
                    &enclosed_view_interest,
                    &surface_interest,
                );
            }
            self.stream_surface_lods(camera.position);
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
            let mut interest =
                crate::urgent_stream_interest(camera, streaming_velocity, lookahead_seconds)
                    .into_iter()
                    .collect::<BTreeSet<_>>();
            let columns = interest
                .iter()
                .map(|coord| (coord.x, coord.z))
                .collect::<BTreeSet<_>>();
            interest.extend(self.surface_hint_stream_interest(&columns));
            interest.into_iter().collect()
        }

        fn movement_collision_interest(
            &self,
            camera: &CameraState,
            streaming_velocity: Vec3,
            lookahead_seconds: f32,
        ) -> Vec<ChunkCoord> {
            let mut interest = crate::movement_stream_interest(
                camera.position,
                streaming_velocity,
                lookahead_seconds,
            )
            .into_iter()
            .collect::<BTreeSet<_>>();
            let columns = interest
                .iter()
                .map(|coord| (coord.x, coord.z))
                .collect::<BTreeSet<_>>();
            interest.extend(self.surface_hint_stream_interest(&columns));
            interest.into_iter().collect()
        }

        fn enclosed_view_stream_interest(&self, camera: &CameraState) -> Vec<ChunkCoord> {
            if self.enclosure.get().enclosure < self.config.stream_enclosed_view_threshold {
                return Vec::new();
            }
            crate::enclosed_view_stream_interest(
                camera,
                self.config.stream_enclosed_view_distance_metres,
                self.config.stream_view_cone_half_angle_degrees,
                &self.chunk_portals.borrow(),
            )
        }

        fn surface_stream_interest(
            &self,
            focus: ChunkCoord,
            collision_interest: &[ChunkCoord],
        ) -> Vec<ChunkCoord> {
            let mut columns = collision_interest
                .iter()
                .map(|coord| (coord.x, coord.z))
                .collect::<BTreeSet<_>>();
            let radius = self.config.startup_ready_radius_chunks;
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    if i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)
                        > i64::from(radius) * i64::from(radius)
                    {
                        continue;
                    }
                    if let (Some(x), Some(z)) = (focus.x.checked_add(dx), focus.z.checked_add(dz)) {
                        columns.insert((x, z));
                    }
                }
            }

            self.surface_hint_stream_interest(&columns)
        }

        fn surface_hint_stream_interest(&self, columns: &BTreeSet<(i32, i32)>) -> Vec<ChunkCoord> {
            let hints = self.surface_chunk_hints.borrow();
            let mut interest = BTreeSet::new();
            for &(chunk_x, chunk_z) in columns {
                let tile = SurfaceTileCoord::new(
                    SurfaceLodLevel::Stride2,
                    chunk_x.div_euclid(2),
                    chunk_z.div_euclid(2),
                );
                let Some(chunk_ys) = hints
                    .get(&tile)
                    .and_then(|tile_hints| tile_hints.get(&(chunk_x, chunk_z)))
                else {
                    continue;
                };
                interest.extend(
                    chunk_ys
                        .iter()
                        .copied()
                        .map(|chunk_y| ChunkCoord::new(chunk_x, chunk_y, chunk_z))
                        .filter(|coord| coord.is_world_representable()),
                );
            }
            interest.into_iter().collect()
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

        fn drain_remote_generation(&self) {
            for completion in self.remote.drain_completions() {
                self.accept_remote_completion(completion);
            }
            for completion in self.remote.drain_surface_completions() {
                self.accept_remote_surface_completion(completion);
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

        fn accept_remote_surface_completion(&self, completion: RemoteSurfaceCompletion) {
            for ticket in &completion.tickets {
                self.surface_in_flight.borrow_mut().remove(&ticket.coord);
            }
            let Ok(result) = completion.result else {
                for ticket in completion.tickets {
                    self.enqueue_surface_front(ticket.coord);
                }
                return;
            };
            if result.source_identity_hash != self.source_identity_hash() {
                log_gpu_error("world surface response identity changed");
                return;
            }
            let mut items = result.items;
            for ticket in completion.tickets {
                let Some(index) = items.iter().position(|item| item.coord == ticket.coord) else {
                    self.enqueue_surface_front(ticket.coord);
                    continue;
                };
                let item = items.remove(index);
                let edit_revision = item.edit_revision;
                let snapshot = match item.result {
                    Ok(snapshot) => snapshot,
                    Err(voxels_world::WorldSourceError::SourceCoverageUnavailable) => continue,
                    Err(error) => {
                        log_gpu_error(&format!(
                            "world service could not generate surface tile {:?}: {error}",
                            ticket.coord
                        ));
                        self.enqueue_surface_front(ticket.coord);
                        continue;
                    }
                };
                let server_floor = self.edit_revisions.borrow().surface_floor(ticket.coord);
                if !revision_satisfies(edit_revision, server_floor)
                    || !self
                        .surface_revisions
                        .borrow()
                        .accepts(ticket.coord, ticket.revision)
                {
                    self.enqueue_surface_front(ticket.coord);
                    continue;
                }
                let canonical_hints = (ticket.coord.level == SurfaceLodLevel::Stride2).then(|| {
                    snapshot
                        .terrain
                        .canonical_surface_chunk_hints(SURFACE_HINT_VERTICAL_MARGIN_CHUNKS)
                });
                if self
                    .renderer
                    .borrow_mut()
                    .upload_surface_tile_meshes(&snapshot.terrain, &snapshot.water)
                {
                    self.surface_resident.borrow_mut().insert(ticket.coord);
                    if let Some(hints) = canonical_hints {
                        self.surface_chunk_hints
                            .borrow_mut()
                            .insert(ticket.coord, hints);
                    }
                    let committed = self
                        .surface_revisions
                        .borrow_mut()
                        .commit(ticket.coord, ticket.revision);
                    debug_assert!(committed, "uploaded remote surface revision became stale");
                    self.surface_accepted_edit_revisions
                        .borrow_mut()
                        .insert(ticket.coord, edit_revision);
                    self.surface_dirty.borrow_mut().remove(&ticket.coord);
                } else {
                    self.enqueue_surface_front(ticket.coord);
                }
            }
        }

        fn enqueue_surface_front(&self, coord: SurfaceTileCoord) {
            if self.surface_in_flight.borrow().contains(&coord)
                || self.surface_queue.borrow().contains(&coord)
            {
                return;
            }
            if usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS {
                self.surface_queue.borrow_mut().push_front(coord);
            } else {
                self.surface_queue.borrow_mut().push_back(coord);
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
            let enclosed_view = complete_interest(enclosed_view_interest);
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
            renderer.set_canonical_ready_chunks(canonical_ready_chunks);
            renderer.set_canonical_surface_ready_chunks(canonical_surface_ready_chunks);
            renderer.set_enclosed_view_ready_chunks(enclosed_view);
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

        fn surface_lod_counts(&self) -> [u32; SURFACE_LOD_LEVEL_COUNT] {
            let mut counts = [0u32; SURFACE_LOD_LEVEL_COUNT];
            for coord in self.surface_resident.borrow().iter() {
                let count = &mut counts[coord.level.index() as usize];
                *count = count.saturating_add(1);
            }
            counts
        }

        fn stream_surface_lods(&self, position: glam::Vec3) {
            let focus = std::array::from_fn(|index| {
                world_to_surface_tile(position, SurfaceLodLevel::ALL[index])
            });
            if self.surface_focus.get() != Some(focus) {
                self.surface_focus.set(Some(focus));
                let mut desired = BTreeSet::new();
                for (index, level) in SurfaceLodLevel::ALL.into_iter().enumerate() {
                    let radius = self.config.surface_load_radius_tiles[index];
                    let level_focus = focus[index];
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            let coord = SurfaceTileCoord::new(
                                level,
                                level_focus.x + dx,
                                level_focus.z + dz,
                            );
                            if coord.is_world_representable() {
                                desired.insert(coord);
                            }
                        }
                    }
                }
                let evicted: Vec<_> = self
                    .surface_resident
                    .borrow()
                    .iter()
                    .copied()
                    .filter(|coord| {
                        let index = coord.level.index() as usize;
                        let retain = self.config.surface_load_radius_tiles[index]
                            + self.config.surface_retain_margin_tiles;
                        let dx = coord.x - focus[index].x;
                        let dz = coord.z - focus[index].z;
                        dx.abs().max(dz.abs()) > retain
                    })
                    .collect();
                if !evicted.is_empty() {
                    let mut resident = self.surface_resident.borrow_mut();
                    let mut revisions = self.surface_revisions.borrow_mut();
                    let mut accepted = self.surface_accepted_edit_revisions.borrow_mut();
                    let mut dirty = self.surface_dirty.borrow_mut();
                    let mut renderer = self.renderer.borrow_mut();
                    for coord in evicted {
                        resident.remove(&coord);
                        self.surface_chunk_hints.borrow_mut().remove(&coord);
                        revisions.evict(coord);
                        accepted.remove(&coord);
                        dirty.remove(&coord);
                        renderer.remove_surface_tile(coord);
                    }
                }

                // Edits may have dirtied a tile just before a focus jump. Keep replacement work only
                // while the tile is still resident or belongs to the new desired set; a future load
                // samples the authoritative edit map and does not need a stale dirty marker.
                {
                    let resident = self.surface_resident.borrow();
                    self.surface_dirty
                        .borrow_mut()
                        .retain(|coord| resident.contains(coord) || desired.contains(coord));
                }

                let resident = self.surface_resident.borrow();
                let mut revisions = self.surface_revisions.borrow_mut();
                let mut dirty = self.surface_dirty.borrow_mut();
                revisions.retain(|coord| resident.contains(&coord) || desired.contains(&coord));
                let mut candidates = Vec::new();
                for coord in desired {
                    match revisions.prepare_focus(coord) {
                        SurfaceFocusAction::Load { .. } => {
                            debug_assert!(!resident.contains(&coord));
                            candidates.push(coord);
                        }
                        SurfaceFocusAction::Replace { .. } => {
                            debug_assert!(resident.contains(&coord));
                            dirty.insert(coord);
                        }
                        SurfaceFocusAction::Current { .. } => {
                            debug_assert!(resident.contains(&coord));
                        }
                    }
                }
                drop(dirty);
                drop(revisions);
                drop(resident);
                let initializing_hierarchy = !self.full_lods_initialized.get();
                candidates.sort_by_key(|coord| {
                    let index = coord.level.index() as usize;
                    let dx = i128::from(coord.x) - i128::from(focus[index].x);
                    let dz = i128::from(coord.z) - i128::from(focus[index].z);
                    let background = index >= INTERACTIVE_SURFACE_LOD_LEVELS;
                    // Startup establishes broad parent cover before refining it. During traversal,
                    // the hierarchy already has retained parents, so finest visible replacements
                    // must win; repeatedly loading each newly exposed coarse strip first otherwise
                    // starves near detail forever at sustained speed.
                    let level_order = if background || !initializing_hierarchy {
                        coord.level.index()
                    } else {
                        u8::MAX - coord.level.index()
                    };
                    (background, level_order, dx * dx + dz * dz, coord.z, coord.x)
                });
                let mut queue = self.surface_queue.borrow_mut();
                queue.clear();
                queue.extend(candidates);
            }

            self.submit_surface_lane(
                WorldProductPriority::VisibleSurface,
                INTERACTIVE_SURFACE_BATCH,
            );
            let bootstrap_ready_for_background = if self.full_lods_initialized.get() {
                true
            } else {
                let diagnostics = self.scheduler.borrow().diagnostics();
                let fine_current = diagnostics.generation.queued == 0
                    && diagnostics.generation.in_flight == 0
                    && diagnostics.meshing.queued == 0
                    && diagnostics.meshing.in_flight == 0
                    && diagnostics.upload.queued == 0
                    && diagnostics.upload.in_flight == 0;
                fine_current
                    && self.surface_coverage_current_through(INTERACTIVE_SURFACE_LOD_LEVELS)
                    && !self.surface_dirty.borrow().iter().any(|coord| {
                        usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS
                    })
                    && !self.surface_in_flight.borrow().iter().any(|coord| {
                        usize::from(coord.level.index()) < INTERACTIVE_SURFACE_LOD_LEVELS
                    })
            };
            if bootstrap_ready_for_background {
                self.submit_surface_lane(WorldProductPriority::Prefetch, BACKGROUND_SURFACE_BATCH);
            }
        }

        fn submit_surface_lane(&self, priority: WorldProductPriority, batch_limit: usize) {
            let background = priority == WorldProductPriority::Prefetch;
            if background
                && self
                    .surface_in_flight
                    .borrow()
                    .iter()
                    .filter(|coord| {
                        usize::from(coord.level.index()) >= INTERACTIVE_SURFACE_LOD_LEVELS
                    })
                    .count()
                    >= BACKGROUND_SURFACE_BATCH * BACKGROUND_SURFACE_BATCHES_IN_FLIGHT
            {
                return;
            }

            let mut tickets = Vec::with_capacity(batch_limit);
            while tickets.len() < batch_limit {
                let position = self.surface_queue.borrow().iter().position(|coord| {
                    (usize::from(coord.level.index()) >= INTERACTIVE_SURFACE_LOD_LEVELS)
                        == background
                });
                let Some(position) = position else {
                    break;
                };
                let Some(coord) = self.surface_queue.borrow_mut().remove(position) else {
                    break;
                };
                if self.surface_in_flight.borrow().contains(&coord)
                    || (self.surface_resident.borrow().contains(&coord)
                        && !self.surface_dirty.borrow().contains(&coord))
                {
                    continue;
                }
                let revision = {
                    let revisions = self.surface_revisions.borrow();
                    revisions
                        .requested_revision(coord)
                        .unwrap_or_else(|| revisions.epoch())
                };
                tickets.push(RemoteSurfaceTicket { coord, revision });
            }
            if tickets.is_empty() {
                return;
            }
            match self.remote.submit_surface_batch(priority, tickets.clone()) {
                Ok(_) => {
                    self.surface_in_flight
                        .borrow_mut()
                        .extend(tickets.into_iter().map(|ticket| ticket.coord));
                }
                Err(
                    RemoteWorldError::Backpressured
                    | RemoteWorldError::RequestWindowFull
                    | RemoteWorldError::NotOpen,
                ) => {
                    let mut queue = self.surface_queue.borrow_mut();
                    for ticket in tickets.into_iter().rev() {
                        queue.push_front(ticket.coord);
                    }
                }
                Err(error) => {
                    let mut queue = self.surface_queue.borrow_mut();
                    for ticket in tickets.into_iter().rev() {
                        queue.push_front(ticket.coord);
                    }
                    log_gpu_error(&format!("submit remote surface batch: {error}"));
                }
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
                        } else if record.code != 7 && !(9..=18).contains(&record.code) {
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
                            &commit.affected_surface_tiles,
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
            affected_surface_tiles: &[SurfaceTileCoord],
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
                affected_surface_tiles,
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
            let surface_revision = if affected_surface_tiles.is_empty() {
                self.surface_revisions.borrow().epoch()
            } else {
                self.surface_revisions.borrow_mut().begin_edit()
            };
            let mut surface = Vec::new();
            for &coord in affected_surface_tiles {
                if !self.surface_tile_relevant(coord)
                    && !self.surface_resident.borrow().contains(&coord)
                {
                    continue;
                }
                self.surface_revisions
                    .borrow_mut()
                    .request(coord, surface_revision);
                self.surface_dirty.borrow_mut().insert(coord);
                self.enqueue_surface_front(coord);
                surface.push(SurfaceRequirement {
                    coord,
                    revision: surface_revision,
                });
            }
            let performance = self.scope.performance();
            let started_ms = performance_now(performance.as_ref());
            let requirements = EditRequirements { canonical, surface };
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
                    surface: requirements.surface.clone(),
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
            self.surface_accepted_edit_revisions.borrow_mut().clear();
            self.pending_meshes.borrow_mut().clear();
            self.pending_uploads.borrow_mut().clear();
            self.canonical_publications.borrow_mut().clear();
            self.scheduler.borrow_mut().invalidate_all_generation();
            let replacement = self.surface_revisions.borrow_mut().begin_edit();
            let retained = self
                .surface_resident
                .borrow()
                .iter()
                .copied()
                .collect::<Vec<_>>();
            for coord in retained {
                self.surface_revisions
                    .borrow_mut()
                    .request(coord, replacement);
                self.surface_dirty.borrow_mut().insert(coord);
                self.enqueue_surface_front(coord);
            }
        }

        fn surface_tile_relevant(&self, coord: SurfaceTileCoord) -> bool {
            coord.is_world_representable()
                && surface_tile_in_coverage(
                    coord,
                    self.surface_focus.get(),
                    self.config.surface_load_radius_tiles,
                )
        }

        fn surface_coverage_current(&self) -> bool {
            self.surface_coverage_current_through(SURFACE_LOD_LEVEL_COUNT)
        }

        fn ready_surface_level_prefix(&self) -> usize {
            let queue = self.surface_queue.borrow();
            let in_flight = self.surface_in_flight.borrow();
            let dirty = self.surface_dirty.borrow();
            (1..=SURFACE_LOD_LEVEL_COUNT)
                .rev()
                .find(|&level_count| {
                    let work_pending = queue
                        .iter()
                        .chain(in_flight.iter())
                        .chain(dirty.iter())
                        .any(|coord| usize::from(coord.level.index()) < level_count);
                    !work_pending && self.surface_coverage_current_through(level_count)
                })
                .unwrap_or(0)
        }

        fn surface_coverage_current_through(&self, level_count: usize) -> bool {
            let Some(focus) = self.surface_focus.get() else {
                return false;
            };
            let resident = self.surface_resident.borrow();
            let revisions = self.surface_revisions.borrow();
            for (index, level) in SurfaceLodLevel::ALL
                .into_iter()
                .take(level_count)
                .enumerate()
            {
                let center = focus[index];
                let radius = self.config.surface_load_radius_tiles[index];
                for dz in -radius..=radius {
                    for dx in -radius..=radius {
                        let coord = SurfaceTileCoord::new(level, center.x + dx, center.z + dz);
                        if !coord.is_world_representable() {
                            continue;
                        }
                        if !resident.contains(&coord) || !revisions.is_current(coord) {
                            return false;
                        }
                    }
                }
            }
            true
        }

        fn update_edit_convergence(&self, now_ms: f64, submitted: bool) {
            if !submitted || self.edit_trackers.borrow().is_empty() {
                return;
            }
            let scheduler = self.scheduler.borrow();
            let surface_revisions = self.surface_revisions.borrow();
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
                let surface_ready = tracker.requirements.surface.iter().all(|requirement| {
                    surface_revisions
                        .resident_revision(requirement.coord)
                        .is_some_and(|revision| revision_satisfies(revision, requirement.revision))
                        || !self.surface_tile_relevant(requirement.coord)
                });
                if canonical_ready && surface_ready {
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
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    impl From<ScreenshotCapture> for MissionControlScreenshot {
        fn from(capture: ScreenshotCapture) -> Self {
            Self {
                filename: capture.filename,
                width: capture.width,
                height: capture.height,
                rgba: capture.rgba,
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

        /// Returns `[tile_x, tile_z, required_server_revision, accepted_server_revision,
        /// resident, dirty, fingerprint_low32, fingerprint_high32, quad_count, activation_mask]`
        /// for the tile containing one canonical voxel coordinate.
        pub fn surface_edit_state(&self, stride: i32, x: i32, z: i32) -> Vec<f64> {
            let Some(engine) = self.engine.as_ref() else {
                return Vec::new();
            };
            let Some(level) = SurfaceLodLevel::from_stride_voxels(stride) else {
                return Vec::new();
            };
            let coord = SurfaceTileCoord::containing(level, x, z);
            let floor = engine.edit_revisions.borrow().surface_floor(coord);
            let accepted = engine
                .surface_accepted_edit_revisions
                .borrow()
                .get(&coord)
                .copied()
                .unwrap_or(0);
            let diagnostics = engine.renderer.borrow().surface_tile_diagnostics(coord);
            let fingerprint = diagnostics.map_or(0, |value| value.0);
            vec![
                f64::from(coord.x),
                f64::from(coord.z),
                floor as f64,
                accepted as f64,
                if engine.surface_resident.borrow().contains(&coord) {
                    1.0
                } else {
                    0.0
                },
                if engine.surface_dirty.borrow().contains(&coord) {
                    1.0
                } else {
                    0.0
                },
                f64::from(fingerprint as u32),
                f64::from((fingerprint >> 32) as u32),
                f64::from(diagnostics.map_or(0, |value| value.1)),
                f64::from(diagnostics.map_or(0, |value| value.2)),
            ]
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
                let (render, target, presented_lod_stride_voxels, canonical_surface_coverage) = {
                    let renderer = engine.renderer.borrow();
                    (
                        renderer.diagnostics(),
                        renderer.target_voxel(),
                        renderer.presented_lod_stride_voxels(
                            camera_voxel_x,
                            camera_voxel_y,
                            camera_voxel_z,
                        ),
                        renderer.canonical_surface_coverage_at(camera_voxel_x, camera_voxel_z),
                    )
                };
                let streaming_velocity = if profile.running() {
                    camera.velocity
                } else {
                    camera.streaming_velocity(&engine.input.borrow())
                };
                let collision_immediate_interest =
                    engine.movement_collision_interest(&camera, streaming_velocity, 0.1);
                let collision_lookahead_seconds =
                    (engine.config.stream_collision_lookahead_seconds
                        - crate::COLLISION_READINESS_RESERVE_SECONDS)
                        .max(0.1);
                let collision_lookahead_interest = engine.movement_collision_interest(
                    &camera,
                    streaming_velocity,
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
                        scheduler.vicinity_readiness(1),
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
                        .filter(|coord| scheduler.desired_chunk_renderable(**coord))
                        .count()
                };
                let edit_canonical_owned = {
                    let renderer = engine.renderer.borrow();
                    edit_canonical_coords
                        .iter()
                        .filter(|coord| renderer.canonical_chunk_owned(**coord))
                        .count()
                };
                let lod_focus_lag_voxels = i64::from(camera_voxel_x)
                    .abs_diff(i64::from(render.lod_boundary_centres[0][0]))
                    .max(
                        i64::from(camera_voxel_z)
                            .abs_diff(i64::from(render.lod_boundary_centres[0][1])),
                    );
                let lod_tiles = engine.surface_lod_counts();
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
                        + diagnostics.upload.in_flight
                        + engine.surface_queue.borrow().len()
                        + engine.surface_in_flight.borrow().len()
                        + engine.surface_dirty.borrow().len()) as f32,
                    engine.surface_resident.borrow().len() as f32,
                    engine.frame_milliseconds.get(),
                    render.shadow_draw_calls as f32,
                    render.shadow_cascades as f32,
                    diagnostics.initial_residency_latency.p95_frames as f32,
                    diagnostics.initial_residency_latency.max_frames as f32,
                    diagnostics.remesh_latency.p95_frames as f32,
                    diagnostics.remesh_latency.max_frames as f32,
                    lod_tiles[0] as f32,
                    lod_tiles[1] as f32,
                    lod_tiles[2] as f32,
                    lod_tiles[3] as f32,
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
                    engine.profile_surface_high.get() as f32,
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
                    if engine.all_lods_ready.get() {
                        1.0
                    } else {
                        0.0
                    },
                    engine.surface_in_flight.borrow().len() as f32,
                    if engine.interactive_lods_ready.get() {
                        1.0
                    } else {
                        0.0
                    },
                    lod_tiles[4] as f32,
                    lod_tiles[5] as f32,
                    lod_tiles[6] as f32,
                    lod_tiles[7] as f32,
                    render.cpu_cull_ms,
                    render.cpu_encode_ms,
                    render.cpu_submit_ms,
                    render.draw_list_tested_slices as f32,
                    render.draw_list_selected_slices as f32,
                    render.surface_width as f32,
                    render.surface_height as f32,
                    render.dpr,
                    render.lod_transition_quads as f32,
                    render.lod_boundary_centres[0][0] as f32,
                    render.lod_boundary_centres[0][1] as f32,
                    render.lod_boundary_centres[1][0] as f32,
                    render.lod_boundary_centres[1][1] as f32,
                    render.lod_boundary_centres[2][0] as f32,
                    render.lod_boundary_centres[2][1] as f32,
                    render.lod_boundary_centres[3][0] as f32,
                    render.lod_boundary_centres[3][1] as f32,
                    render.lod_boundary_centres[4][0] as f32,
                    render.lod_boundary_centres[4][1] as f32,
                    render.lod_boundary_centres[5][0] as f32,
                    render.lod_boundary_centres[5][1] as f32,
                    render.lod_boundary_centres[6][0] as f32,
                    render.lod_boundary_centres[6][1] as f32,
                    render.lod_boundary_centres[7][0] as f32,
                    render.lod_boundary_centres[7][1] as f32,
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
                    f32::from(presented_lod_stride_voxels),
                    lod_focus_lag_voxels as f32,
                    canonical_immediate.resident as f32,
                    canonical_immediate.required as f32,
                    f32::from(canonical_surface_coverage.0),
                    f32::from(canonical_surface_coverage.1),
                    diagnostics.generation.queued as f32,
                    diagnostics.generation.in_flight as f32,
                    diagnostics.meshing.queued as f32,
                    diagnostics.meshing.in_flight as f32,
                    diagnostics.upload.queued as f32,
                    diagnostics.upload.in_flight as f32,
                    engine.surface_queue.borrow().len() as f32,
                    engine.surface_dirty.borrow().len() as f32,
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
            stream_enclosed_view_threshold: streaming.priority.enclosed_view_threshold,
            surface_load_radius_tiles: streaming
                .surface
                .load_radius_tiles
                .map(|radius| radius as i32),
            surface_retain_margin_tiles: streaming.surface.retention_margin_tiles as i32,
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
        let camera = crate::camera_from_resume_values([
            resume.eye_position_metres[0],
            resume.eye_position_metres[1],
            resume.eye_position_metres[2],
            resume.look_yaw_radians,
            resume.look_pitch_radians,
        ]);
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
            camera: RefCell::new(camera),
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
            surface_focus: Cell::new(None),
            surface_resident: RefCell::new(BTreeSet::new()),
            surface_chunk_hints: RefCell::new(BTreeMap::new()),
            surface_revisions: RefCell::new(SurfaceRevisionCache::new()),
            surface_accepted_edit_revisions: RefCell::new(BTreeMap::new()),
            surface_queue: RefCell::new(VecDeque::new()),
            surface_in_flight: RefCell::new(BTreeSet::new()),
            surface_dirty: RefCell::new(BTreeSet::new()),
            all_lods_ready: Cell::new(false),
            interactive_lods_ready: Cell::new(false),
            full_lods_initialized: Cell::new(false),
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
            profile_surface_high: Cell::new(0),
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

    fn world_to_surface_tile(position: glam::Vec3, level: SurfaceLodLevel) -> SurfaceTileCoord {
        let voxel_x = (position.x / VOXEL_SIZE_METRES).floor() as i32;
        let voxel_z = (position.z / VOXEL_SIZE_METRES).floor() as i32;
        SurfaceTileCoord::containing(level, voxel_x, voxel_z)
    }

    fn surface_tile_in_coverage(
        coord: SurfaceTileCoord,
        focus: Option<[SurfaceTileCoord; SURFACE_LOD_LEVEL_COUNT]>,
        load_radius_tiles: [i32; SURFACE_LOD_LEVEL_COUNT],
    ) -> bool {
        let Some(focus) = focus else {
            return false;
        };
        let index = coord.level.index() as usize;
        let center = focus[index];
        let dx = (coord.x - center.x).abs();
        let dz = (coord.z - center.z).abs();
        dx.max(dz) <= load_radius_tiles[index]
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
    fn enclosed_view_interest_reaches_beyond_the_surface_handoff() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let portals = straight_tunnel_portals(-9, 0);
        let interest = enclosed_view_stream_interest(&camera, 32.0, 55.0, &portals);

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
    fn enclosed_view_interest_owns_nearby_occluders_without_streaming_behind_them() {
        let camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        let portals = BTreeMap::from([
            ((0, 1, 0), portal_mask(&[4, 5])),
            ((0, 1, -1), portal_mask(&[])),
        ]);
        let interest = enclosed_view_stream_interest(&camera, 32.0, 55.0, &portals);

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
        let first = enclosed_view_stream_interest(&camera, 24.0, 55.0, &portals);

        camera.yaw += 0.55;
        let turned = enclosed_view_stream_interest(&camera, 24.0, 55.0, &portals);

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
    fn enclosed_view_interest_does_not_follow_connected_air_behind_the_camera() {
        let mut camera = CameraState::spawn(glam::Vec3::new(1.6, 3.25, 1.6));
        camera.yaw = std::f32::consts::PI;
        let portals = straight_tunnel_portals(-9, 0);
        let interest = enclosed_view_stream_interest(&camera, 32.0, 55.0, &portals);

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

        let interest = enclosed_view_stream_interest(&camera, 32.0, 55.0, &portals);

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

        let interest = enclosed_view_stream_interest(&camera, 32.0, 55.0, &portals);

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
