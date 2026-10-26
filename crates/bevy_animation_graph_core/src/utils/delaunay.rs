use bevy::{math::Vec2, reflect::Reflect};

use super::geometry::{CachedTriangle, Triangle, Vertex, VertexId};

#[derive(Default, Reflect, Clone, Debug)]
pub struct Triangulation {
    triangles: Vec<CachedTriangle>,
}

impl Triangulation {
    pub fn from_points_delaunay(points: Vec<Vec2>) -> Self {
        let vertices = points
            .into_iter()
            .enumerate()
            .map(|(i, v)| Vertex::new(v, VertexId::Index(i)))
            .collect::<Vec<_>>();
        let super_triangle = Triangle::super_triangle(&vertices);
        let mut triangles = vec![CachedTriangle::from_triangle(super_triangle.clone())];
        for vertex in vertices {
            triangles = add_vertex(triangles, vertex);
        }

        Triangulation {
            triangles: triangles
                .into_iter()
                .filter(|triangle| triangle.inner().all_vertices_have_index())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    pub fn find_linear_combination(&self, p: Vec2) -> Vec<(Vertex, f32)> {
        let (i, (closest_p, _)) = self
            .triangles
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.distance_to_point(p)))
            .min_by(|(_, (_, d0)), (_, (_, d1))| d0.partial_cmp(d1).unwrap())
            .unwrap();

        let triangle = self.triangles[i].inner();
        let bary = triangle.barycentric_coordinates(closest_p);

        vec![
            (triangle.p, bary.x),
            (triangle.q, bary.y),
            (triangle.r, bary.z),
        ]
    }
}

fn add_vertex(mut triangles: Vec<CachedTriangle>, vertex: Vertex) -> Vec<CachedTriangle> {
    let mut edges = vec![];

    triangles.retain(|triangle| {
        if triangle.in_circumcircle(vertex.val).unwrap_or(true) {
            edges.extend(triangle.inner().edges());
            false
        } else {
            true
        }
    });

    // The hole's boundary: edges of exactly one removed triangle. An edge two removed
    // triangles share is inside the hole and must go.
    let edges: Vec<_> = edges
        .iter()
        .filter(|e| edges.iter().filter(|o| e.compare_value(o)).count() == 1)
        .cloned()
        .collect();

    edges.into_iter().for_each(|edge| {
        triangles.push(CachedTriangle::from_triangle(Triangle::new(
            edge.p, edge.q, vertex,
        )))
    });

    triangles
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_basic_triangulation() {
        let triangulation = Triangulation::from_points_delaunay(vec![
            Vec2::new(0., 0.),
            Vec2::new(1., 0.),
            Vec2::new(0., 1.),
        ]);

        assert_eq!(triangulation.triangles.len(), 1);
    }

    #[test]
    fn test_linear_combination() {
        let triangulation = Triangulation::from_points_delaunay(vec![
            Vec2::new(0., 0.),
            Vec2::new(1., 0.),
            Vec2::new(0., 1.),
        ]);

        let linear_combination = triangulation.find_linear_combination(Vec2::new(0.1, 0.1));

        assert_eq!(
            1.,
            linear_combination[0].1 + linear_combination[1].1 + linear_combination[2].1
        );
    }
}
#[cfg(test)]
mod walk_space {
    use super::*;
    use crate::utils::geometry::VertexId;

    /// A walk/strafe/backpedal space: a few triangles, and a point past the back edge lands
    /// wholly on the backpedal, not on a forward clip.
    #[test]
    fn a_point_outside_projects_onto_the_nearest_edge() {
        let points = [
            (0., 0.),
            (0., 1.0),
            (-0.75, 0.75),
            (0.75, 0.75),
            (-0.7, 0.),
            (0.6, 0.),
            (0., -1.05),
            (-0.975, -0.975),
            (0.975, -0.975),
            (0., 5.4),
            (0., 8.25),
        ];
        let t = Triangulation::from_points_delaunay(
            points.iter().map(|&(x, y)| Vec2::new(x, y)).collect(),
        );
        assert!(
            t.triangles.len() < 2 * points.len(),
            "{} triangles",
            t.triangles.len()
        );
        let back = t
            .find_linear_combination(Vec2::new(0., -3.))
            .into_iter()
            .find(|(v, _)| v.id == VertexId::Index(6))
            .map(|(_, w)| w);
        assert!(back.is_some_and(|w| w > 0.99), "{back:?}");
    }
}
