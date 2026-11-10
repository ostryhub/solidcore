//! Seeded modelling commands and cache-only inspection for a caller that owns validation policy.

use solidcore::geom::extent3d::Extent3d;
use solidcore::mass::integrator::properties::{brep_volume_default, no_unsure_region};
use solidcore::nurbs::b_spline_curve::BoundingBoxRequest;
use solidcore::primitive::api_primitives;
use solidcore::query::api_queries::brep_bounding_box;
use solidcore::query::{edge, face};
use solidcore::topology::brep::{mirror::transform_matrix, p1, p5};
use solidcore::topology::vertex;
use solidcore::{Brep, Vec3};

use crate::body::Body;
use crate::session::{
    copy_operand, read_body, require_finished, run, run_on, seed_command, with_cached_body,
};
use crate::{Error, Order, Result};

type Bounds = ([f64; 3], [f64; 3]);

impl Body {
    /// Makes a seeded axis-aligned box from validated dimensions.
    pub fn seeded_box(origin: [f64; 3], size: [f64; 3]) -> Result<Self> {
        run("create_box", |kernel| {
            seed_command(kernel);
            api_primitives::create_box(kernel, vec(origin), size[0], size[1], size[2])
        })
        .map(Self::new)
    }

    /// Makes a seeded sphere from a validated center and radius.
    pub fn seeded_sphere(center: [f64; 3], radius: f64) -> Result<Self> {
        run("create_sphere", |kernel| {
            seed_command(kernel);
            api_primitives::create_sphere(kernel, vec(center), radius)
        })
        .map(Self::new)
    }

    /// Makes and rigidly places a cylinder within one seeded command.
    pub fn placed_cylinder(radius: f64, height: f64, placement: &[[f64; 4]; 4]) -> Result<Self> {
        run("create_cylinder", |kernel| {
            seed_command(kernel);
            let mut body = api_primitives::create_cylinder(kernel, vec([0.0; 3]), radius, height)?;
            transform_matrix(kernel, &mut body, placement, false)?;
            Ok(body)
        })
        .map(Self::new)
    }

    /// Makes and rigidly places a cone. The optional first map reverses a cone whose
    /// requested base is its apex. Both maps run under the creation command's seed.
    pub fn placed_cone(
        base_radius: f64,
        top_radius: f64,
        height: f64,
        reverse: Option<&[[f64; 4]; 4]>,
        placement: &[[f64; 4]; 4],
    ) -> Result<Self> {
        run("create_cone", |kernel| {
            seed_command(kernel);
            let mut body = api_primitives::create_cone(
                kernel,
                vec([0.0; 3]),
                base_radius,
                top_radius,
                height,
            )?;
            if let Some(reverse) = reverse {
                transform_matrix(kernel, &mut body, reverse, false)?;
            }
            transform_matrix(kernel, &mut body, placement, false)?;
            Ok(body)
        })
        .map(Self::new)
    }

    /// Rigidly transforms a copy of this body using a validated placement matrix.
    pub fn transformed_rigid(&self, matrix: &[[f64; 4]; 4]) -> Result<Self> {
        let call = "transform_rigid";
        run_on(call, &[("brep", self)], |kernel| {
            if let Err(error) = require_finished(self, call) {
                return Ok(Err(error));
            }
            seed_command(kernel);
            let mut body = copy_operand(kernel, self)?;
            if !is_empty(&body) {
                transform_matrix(kernel, &mut body, matrix, false)?;
            }
            Ok(Ok(Self::new(body)))
        })?
    }

    /// Unifies the requested topology on a seeded copy. A face-only refinement
    /// keeps the copied vertices, and an edge-only refinement skips loop vertices.
    pub fn refined_topology(&self, unify_edges: bool, unify_faces: bool) -> Result<Self> {
        let call = "refine_topology";
        run_on(call, &[("brep", self)], |kernel| {
            if let Err(error) = require_finished(self, call) {
                return Ok(Err(error));
            }
            seed_command(kernel);
            let mut body = copy_operand(kernel, self)?;
            if is_empty(&body) {
                return Ok(Ok(Self::new(body)));
            }
            if unify_faces {
                let vertices = p1::vertices(&body, None)?;
                let keep = (!unify_edges).then_some(vertices.as_slice());
                p5::remove_topological_edges_and_vertices(kernel, &mut body, None, keep)?;
            } else if unify_edges {
                for vertex in p1::vertices(&body, None)? {
                    if vertex::is_loop_vertex(&body, vertex)? {
                        continue;
                    }
                    if vertex::is_topological_vertex(kernel, &mut body, vertex)? {
                        p5::delete_topological_vertex(kernel, &mut body, true, Some(vertex), None)?;
                    }
                }
            }
            body.refresh_cached_topology();
            Ok(Ok(Self::new(body)))
        })?
    }

    /// Reads the volume with the kernel's default integration attributes.
    /// Successful empty bodies have zero volume; unsure material regions are refused.
    pub fn volume_without_attributes(&self, relative_accuracy: f64) -> Result<f64> {
        with_cached_body("volume_without_attributes", self, |kernel, body| {
            seed_command(kernel);
            if is_empty(body) {
                return Ok(0.0);
            }
            no_unsure_region(body)?;
            brep_volume_default(kernel, body, relative_accuracy)
        })
    }

    /// Reads the native tight box or no bounds for an empty body.
    pub fn tight_bounds(&self) -> Result<Option<Bounds>> {
        read_body("tight_bounds", self, |kernel, body| {
            if is_empty(body) {
                return Ok(None);
            }
            brep_bounding_box(kernel, body, true).map(|bounds| Some(corners(&bounds)))
        })
    }

    /// Reads the native bounds of selected faces, edges and vertices in this body's order.
    pub fn bounds_for_order(
        &self,
        order: &Order,
        faces: &[usize],
        edges: &[usize],
        vertices: &[usize],
    ) -> Result<Option<Bounds>> {
        with_cached_body("bounds_for_order", self, |kernel, body| {
            seed_command(kernel);
            let mut bounds = Extent3d::new();
            let mut has_geometry = false;
            for &index in faces {
                let Some(&id) = order.faces.get(index) else {
                    return Ok(Err(Error::Argument));
                };
                let extent = face::p1::calculate_bounding_box(kernel, &mut *body, id, true)?;
                add_extent(&mut bounds, &extent);
                has_geometry = true;
            }
            let request = BoundingBoxRequest {
                normal: true,
                pseudo: false,
                polar: false,
            };
            for &index in edges {
                let Some(&id) = order.edges.get(index) else {
                    return Ok(Err(Error::Argument));
                };
                let extent = edge::calculate_bounding_box(kernel, body, id, request, true)?;
                if let Some(extent) = extent.normal {
                    add_extent(&mut bounds, &extent);
                    has_geometry = true;
                }
            }
            for &index in vertices {
                let Some(&id) = order.vertices.get(index) else {
                    return Ok(Err(Error::Argument));
                };
                bounds.add_point(vertex::point(body, id)?);
                has_geometry = true;
            }
            Ok(Ok(has_geometry.then(|| corners(&bounds))))
        })?
    }
}

fn is_empty(body: &Brep) -> bool {
    body.faces().is_empty()
}

fn vec([x, y, z]: [f64; 3]) -> Vec3 {
    Vec3::new(x, y, z)
}

fn corners(extent: &Extent3d) -> Bounds {
    (
        std::array::from_fn(|i| extent.min_at(i)),
        std::array::from_fn(|i| extent.max_at(i)),
    )
}

fn add_extent(target: &mut Extent3d, value: &Extent3d) {
    let (min, max) = corners(value);
    target.add_point(vec(min));
    target.add_point(vec(max));
}
