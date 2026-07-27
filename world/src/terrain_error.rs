//! Independent geometric error certification for simplified terrain pages.
//!
//! The simplifier reports its own collapse error, but publication must not trust the producer it
//! is verifying. This module measures both directed surface distances against an independently
//! built triangle BVH. Each source triangle is sampled on a deterministic barycentric lattice and
//! the lattice cell diameter is added to the largest observed distance. Distance to a closed set
//! is 1-Lipschitz, so that sum is a conservative upper bound for every unsampled point.

use crate::{TerrainClusterTriangle, TerrainPageBuildError, TerrainTriangleCluster};

const LEAF_TRIANGLES: usize = 8;
const MAX_SUBDIVISIONS_PER_EDGE: usize = 2_048;
const MAX_SAMPLES_PER_DIRECTION: usize = 24_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CertifiedSurfaceError {
    pub upper_bound_millivoxels: u32,
    pub samples: usize,
}

pub(crate) fn certify_bidirectional_surface_error(
    exact: &TerrainTriangleCluster,
    simplified: &TerrainTriangleCluster,
    origin: [i32; 3],
    sample_spacing_millivoxels: u32,
    maximum_error_millivoxels: u32,
) -> Result<CertifiedSurfaceError, TerrainPageBuildError> {
    if sample_spacing_millivoxels == 0
        || maximum_error_millivoxels == 0
        || exact.triangles.is_empty()
        || simplified.triangles.is_empty()
    {
        return Err(TerrainPageBuildError::InvalidSimplification);
    }
    let exact_triangles = cluster_triangles(exact, origin)?;
    let simplified_triangles = cluster_triangles(simplified, origin)?;
    let exact_bvh = TriangleBvh::build(exact_triangles.clone())?;
    let simplified_bvh = TriangleBvh::build(simplified_triangles.clone())?;
    let spacing = f64::from(sample_spacing_millivoxels) / 1_000.0;
    let maximum = f64::from(maximum_error_millivoxels) / 1_000.0;

    let exact_to_simplified =
        directed_surface_error(&exact_triangles, &simplified_bvh, spacing, maximum)?;
    let simplified_to_exact =
        directed_surface_error(&simplified_triangles, &exact_bvh, spacing, maximum)?;
    let upper_bound_voxels = exact_to_simplified
        .upper_bound_voxels
        .max(simplified_to_exact.upper_bound_voxels);
    let upper_bound_millivoxels = (upper_bound_voxels * 1_000.0)
        .ceil()
        .clamp(0.0, f64::from(u32::MAX)) as u32;
    if upper_bound_millivoxels > maximum_error_millivoxels {
        return Err(TerrainPageBuildError::InvalidSimplification);
    }
    Ok(CertifiedSurfaceError {
        upper_bound_millivoxels,
        samples: exact_to_simplified
            .samples
            .saturating_add(simplified_to_exact.samples),
    })
}

#[derive(Clone, Copy, Debug)]
struct DirectedSurfaceError {
    upper_bound_voxels: f64,
    samples: usize,
}

fn directed_surface_error(
    source: &[Triangle],
    target: &TriangleBvh,
    spacing: f64,
    maximum: f64,
) -> Result<DirectedSurfaceError, TerrainPageBuildError> {
    let mut maximum_upper = 0.0f64;
    let mut samples = 0usize;
    for triangle in source {
        let maximum_edge = triangle.maximum_edge_length();
        let subdivisions = ((maximum_edge / spacing).ceil() as usize).max(1);
        if subdivisions > MAX_SUBDIVISIONS_PER_EDGE {
            return Err(TerrainPageBuildError::InvalidSimplification);
        }
        let triangle_samples = subdivisions
            .checked_add(1)
            .and_then(|edge| edge.checked_mul(edge + 1))
            .map(|product| product / 2)
            .ok_or(TerrainPageBuildError::InvalidSimplification)?;
        samples = samples
            .checked_add(triangle_samples)
            .filter(|count| *count <= MAX_SAMPLES_PER_DIRECTION)
            .ok_or(TerrainPageBuildError::InvalidSimplification)?;

        let denominator = subdivisions as f64;
        let mut sampled_maximum_squared = 0.0f64;
        for first in 0..=subdivisions {
            for second in 0..=subdivisions - first {
                let first_weight = first as f64 / denominator;
                let second_weight = second as f64 / denominator;
                let point = add(
                    triangle.points[0],
                    add(
                        scale(
                            subtract(triangle.points[1], triangle.points[0]),
                            first_weight,
                        ),
                        scale(
                            subtract(triangle.points[2], triangle.points[0]),
                            second_weight,
                        ),
                    ),
                );
                sampled_maximum_squared =
                    sampled_maximum_squared.max(target.nearest_distance_squared(point));
            }
        }
        let covering_radius = maximum_edge / denominator;
        let upper = sampled_maximum_squared.sqrt() + covering_radius;
        if !upper.is_finite() || upper > maximum {
            return Err(TerrainPageBuildError::InvalidSimplification);
        }
        maximum_upper = maximum_upper.max(upper);
    }
    Ok(DirectedSurfaceError {
        upper_bound_voxels: maximum_upper,
        samples,
    })
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    points: [[f64; 3]; 3],
    bounds: Aabb,
    centroid: [f64; 3],
}

impl Triangle {
    fn new(points: [[f64; 3]; 3]) -> Self {
        let bounds = Aabb::from_points(points);
        let centroid = scale(add(add(points[0], points[1]), points[2]), 1.0 / 3.0);
        Self {
            points,
            bounds,
            centroid,
        }
    }

    fn maximum_edge_length(self) -> f64 {
        distance_squared(self.points[0], self.points[1])
            .max(distance_squared(self.points[1], self.points[2]))
            .max(distance_squared(self.points[2], self.points[0]))
            .sqrt()
    }

    fn distance_squared(self, point: [f64; 3]) -> f64 {
        point_triangle_distance_squared(point, self.points)
    }
}

fn cluster_triangles(
    cluster: &TerrainTriangleCluster,
    origin: [i32; 3],
) -> Result<Vec<Triangle>, TerrainPageBuildError> {
    cluster
        .triangles
        .iter()
        .map(|triangle| cluster_triangle(cluster, *triangle, origin))
        .collect()
}

fn cluster_triangle(
    cluster: &TerrainTriangleCluster,
    triangle: TerrainClusterTriangle,
    origin: [i32; 3],
) -> Result<Triangle, TerrainPageBuildError> {
    let mut points = [[0.0; 3]; 3];
    for (destination, index) in points.iter_mut().zip(triangle.vertices) {
        let vertex = cluster
            .vertices
            .get(index as usize)
            .ok_or(TerrainPageBuildError::InvalidSimplification)?;
        *destination = [
            f64::from(vertex.position[0] - origin[0]),
            f64::from(vertex.position[1] - origin[1]),
            f64::from(vertex.position[2] - origin[2]),
        ];
    }
    Ok(Triangle::new(points))
}

#[derive(Clone, Copy, Debug)]
struct Aabb {
    minimum: [f64; 3],
    maximum: [f64; 3],
}

impl Aabb {
    fn empty() -> Self {
        Self {
            minimum: [f64::INFINITY; 3],
            maximum: [f64::NEG_INFINITY; 3],
        }
    }

    fn from_points(points: [[f64; 3]; 3]) -> Self {
        let mut bounds = Self::empty();
        for point in points {
            bounds.include_point(point);
        }
        bounds
    }

    fn include_point(&mut self, point: [f64; 3]) {
        for (axis, value) in point.into_iter().enumerate() {
            self.minimum[axis] = self.minimum[axis].min(value);
            self.maximum[axis] = self.maximum[axis].max(value);
        }
    }

    fn include_bounds(&mut self, other: Self) {
        self.include_point(other.minimum);
        self.include_point(other.maximum);
    }

    fn distance_squared(self, point: [f64; 3]) -> f64 {
        (0..3)
            .map(|axis| {
                let distance = if point[axis] < self.minimum[axis] {
                    self.minimum[axis] - point[axis]
                } else if point[axis] > self.maximum[axis] {
                    point[axis] - self.maximum[axis]
                } else {
                    0.0
                };
                distance * distance
            })
            .sum()
    }

    fn longest_centroid_axis(triangles: &[Triangle], indices: &[usize]) -> usize {
        let mut centroid_bounds = Self::empty();
        for index in indices {
            centroid_bounds.include_point(triangles[*index].centroid);
        }
        (1..3).fold(0, |longest, axis| {
            let length = centroid_bounds.maximum[axis] - centroid_bounds.minimum[axis];
            let longest_length =
                centroid_bounds.maximum[longest] - centroid_bounds.minimum[longest];
            if length > longest_length {
                axis
            } else {
                longest
            }
        })
    }
}

#[derive(Clone, Debug)]
enum BvhNode {
    Leaf { bounds: Aabb, triangles: Vec<usize> },
    Branch { bounds: Aabb, children: [usize; 2] },
}

impl BvhNode {
    fn bounds(&self) -> Aabb {
        match self {
            Self::Leaf { bounds, .. } | Self::Branch { bounds, .. } => *bounds,
        }
    }
}

#[derive(Clone, Debug)]
struct TriangleBvh {
    triangles: Vec<Triangle>,
    nodes: Vec<BvhNode>,
    root: usize,
}

impl TriangleBvh {
    fn build(triangles: Vec<Triangle>) -> Result<Self, TerrainPageBuildError> {
        if triangles.is_empty() {
            return Err(TerrainPageBuildError::InvalidSimplification);
        }
        let mut bvh = Self {
            triangles,
            nodes: Vec::new(),
            root: 0,
        };
        let mut indices = (0..bvh.triangles.len()).collect::<Vec<_>>();
        bvh.root = bvh.build_node(&mut indices);
        Ok(bvh)
    }

    fn build_node(&mut self, indices: &mut [usize]) -> usize {
        let mut bounds = Aabb::empty();
        for index in indices.iter().copied() {
            bounds.include_bounds(self.triangles[index].bounds);
        }
        if indices.len() <= LEAF_TRIANGLES {
            let node = self.nodes.len();
            self.nodes.push(BvhNode::Leaf {
                bounds,
                triangles: indices.to_vec(),
            });
            return node;
        }
        let axis = Aabb::longest_centroid_axis(&self.triangles, indices);
        indices.sort_unstable_by(|left, right| {
            self.triangles[*left].centroid[axis]
                .total_cmp(&self.triangles[*right].centroid[axis])
                .then_with(|| left.cmp(right))
        });
        let middle = indices.len() / 2;
        let (left, right) = indices.split_at_mut(middle);
        let children = [self.build_node(left), self.build_node(right)];
        let node = self.nodes.len();
        self.nodes.push(BvhNode::Branch { bounds, children });
        node
    }

    fn nearest_distance_squared(&self, point: [f64; 3]) -> f64 {
        self.nearest_in_node(self.root, point, f64::INFINITY)
    }

    fn nearest_in_node(&self, node: usize, point: [f64; 3], mut best: f64) -> f64 {
        let current = &self.nodes[node];
        if current.bounds().distance_squared(point) >= best {
            return best;
        }
        match current {
            BvhNode::Leaf { triangles, .. } => {
                for index in triangles {
                    best = best.min(self.triangles[*index].distance_squared(point));
                }
            }
            BvhNode::Branch { children, .. } => {
                let mut ordered = *children;
                if self.nodes[ordered[1]].bounds().distance_squared(point)
                    < self.nodes[ordered[0]].bounds().distance_squared(point)
                {
                    ordered.swap(0, 1);
                }
                best = self.nearest_in_node(ordered[0], point, best);
                best = self.nearest_in_node(ordered[1], point, best);
            }
        }
        best
    }
}

fn point_triangle_distance_squared(point: [f64; 3], triangle: [[f64; 3]; 3]) -> f64 {
    // Real-Time Collision Detection, Christer Ericson, closest point on triangle.
    let [a, b, c] = triangle;
    let ab = subtract(b, a);
    let ac = subtract(c, a);
    let ap = subtract(point, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return distance_squared(point, a);
    }

    let bp = subtract(point, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return distance_squared(point, b);
    }

    let vertex_c_region = d1 * d4 - d3 * d2;
    if vertex_c_region <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let weight = d1 / (d1 - d3);
        return distance_squared(point, add(a, scale(ab, weight)));
    }

    let cp = subtract(point, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return distance_squared(point, c);
    }

    let vertex_b_region = d5 * d2 - d1 * d6;
    if vertex_b_region <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let weight = d2 / (d2 - d6);
        return distance_squared(point, add(a, scale(ac, weight)));
    }

    let edge_bc_region = d3 * d6 - d5 * d4;
    if edge_bc_region <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let weight = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return distance_squared(point, add(b, scale(subtract(c, b), weight)));
    }

    let denominator = 1.0 / (edge_bc_region + vertex_b_region + vertex_c_region);
    let b_weight = vertex_b_region * denominator;
    let c_weight = vertex_c_region * denominator;
    let closest = add(a, add(scale(ab, b_weight), scale(ac, c_weight)));
    distance_squared(point, closest)
}

fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(vector: [f64; 3], scale: f64) -> [f64; 3] {
    [vector[0] * scale, vector[1] * scale, vector[2] * scale]
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn distance_squared(left: [f64; 3], right: [f64; 3]) -> f64 {
    dot(subtract(left, right), subtract(left, right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TerrainClusterVertex;

    fn cluster(points: &[[i32; 3]]) -> TerrainTriangleCluster {
        TerrainTriangleCluster {
            vertices: points
                .iter()
                .copied()
                .map(|position| TerrainClusterVertex {
                    position,
                    material_index: 0,
                })
                .collect(),
            triangles: vec![TerrainClusterTriangle {
                vertices: [0, 1, 2],
                material_index: 0,
            }],
        }
    }

    #[test]
    fn identical_triangles_receive_only_the_lattice_covering_margin() {
        let surface = cluster(&[[0, 0, 0], [4, 0, 0], [0, 4, 0]]);
        let certificate =
            certify_bidirectional_surface_error(&surface, &surface, [0; 3], 250, 250).unwrap();
        assert!((245..=250).contains(&certificate.upper_bound_millivoxels));
        assert!(certificate.samples > 0);
    }

    #[test]
    fn displaced_triangle_is_rejected_above_the_claimed_error() {
        let exact = cluster(&[[0, 0, 0], [4, 0, 0], [0, 4, 0]]);
        let displaced = cluster(&[[0, 1, 0], [4, 1, 0], [0, 5, 0]]);
        assert_eq!(
            certify_bidirectional_surface_error(&exact, &displaced, [0; 3], 250, 1_000),
            Err(TerrainPageBuildError::InvalidSimplification)
        );
        let certificate =
            certify_bidirectional_surface_error(&exact, &displaced, [0; 3], 250, 1_250).unwrap();
        assert!((1_245..=1_250).contains(&certificate.upper_bound_millivoxels));
    }

    #[test]
    fn triangle_distance_handles_face_edge_and_vertex_regions() {
        let triangle = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        assert_eq!(
            point_triangle_distance_squared([0.5, 0.5, 2.0], triangle),
            4.0
        );
        assert_eq!(
            point_triangle_distance_squared([1.5, 1.5, 0.0], triangle),
            0.5
        );
        assert_eq!(
            point_triangle_distance_squared([-1.0, -1.0, 0.0], triangle),
            2.0
        );
    }
}
