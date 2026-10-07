//! Worlds and collisions (sim-design 4.7): static colliders with per-material restitution
//! and friction, the quad's collider (duct rings and a body box for a whoop; a body box and
//! prop-disc sensors for an open-prop quad), and collider streaming within a radius.
//!
//! Rapier (Apache-2.0) does the contact solving, with its deterministic build. Collider
//! streaming and prop-strike sensors follow propwash's approach (MIT,
//! `LICENSES/propwash.txt`).

use rapier3d_f64::glamx::{DQuat, DVec3};
use rapier3d_f64::prelude::*;
use serde::{Deserialize, Serialize};

use crate::profile::Params;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Material {
    Wall,
    Floor,
    Carpet,
    Grass,
    Gate,
}

impl Material {
    /// (restitution, friction).
    pub fn coefficients(self) -> (f64, f64) {
        match self {
            Material::Wall => (0.45, 0.4),
            Material::Floor => (0.35, 0.5),
            Material::Carpet => (0.15, 0.9),
            Material::Grass => (0.1, 0.8),
            Material::Gate => (0.4, 0.3),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum Shape {
    Box {
        half: [f64; 3],
    },
    Mesh {
        vertices: Vec<[f64; 3]>,
        triangles: Vec<[u32; 3]>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldCollider {
    #[serde(flatten)]
    pub shape: Shape,
    pub position: [f64; 3],
    /// Rotation about the vertical axis (deg).
    #[serde(default)]
    pub yaw_deg: f64,
    pub material: Material,
}

impl WorldCollider {
    /// A bounding radius around its position (for streaming).
    fn radius(&self) -> f64 {
        match &self.shape {
            Shape::Box { half } => {
                (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt()
            }
            Shape::Mesh { vertices, .. } => vertices
                .iter()
                .map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
                .fold(0.0, f64::max),
        }
    }

    fn build(&self) -> Collider {
        let (rest, fric) = self.material.coefficients();
        let b = match &self.shape {
            Shape::Box { half } => ColliderBuilder::cuboid(half[0], half[1], half[2]),
            Shape::Mesh {
                vertices,
                triangles,
            } => {
                let v = vertices
                    .iter()
                    .map(|p| DVec3::new(p[0], p[1], p[2]))
                    .collect();
                ColliderBuilder::trimesh(v, triangles.clone())
                    .expect("world mesh: invalid triangles")
            }
        };
        b.translation(DVec3::from(self.position))
            .rotation(DVec3::new(0.0, 0.0, self.yaw_deg.to_radians()))
            .restitution(rest)
            .friction(fric)
            .restitution_combine_rule(CoefficientCombineRule::Average)
            .build()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSpec {
    pub name: String,
    pub colliders: Vec<WorldCollider>,
    /// The start pad: where a reset puts the quad (floor level), and its heading (deg).
    pub start: [f64; 3],
    pub start_yaw_deg: f64,
    /// Colliders within this distance of the quad are live (m).
    pub stream_radius: f64,
}

impl WorldSpec {
    /// No colliders at all: free flight for physics tests.
    pub fn empty() -> WorldSpec {
        WorldSpec {
            name: "empty".into(),
            colliders: Vec::new(),
            start: [0.0; 3],
            start_yaw_deg: 0.0,
            stream_radius: 50.0,
        }
    }

    /// A box room: floor, four walls and a ceiling, inside dimensions `w` × `d` × `h` (m),
    /// floor at z = 0, start pad in the middle.
    pub fn plain_room(w: f64, d: f64, h: f64) -> WorldSpec {
        let t = 0.1;
        let bx = |half: [f64; 3], position: [f64; 3], material| WorldCollider {
            shape: Shape::Box { half },
            position,
            yaw_deg: 0.0,
            material,
        };
        WorldSpec {
            name: "plain room".into(),
            colliders: vec![
                bx(
                    [w / 2.0 + t, d / 2.0 + t, t],
                    [0.0, 0.0, -t],
                    Material::Floor,
                ),
                bx(
                    [w / 2.0 + t, d / 2.0 + t, t],
                    [0.0, 0.0, h + t],
                    Material::Wall,
                ),
                bx(
                    [t, d / 2.0, h / 2.0],
                    [w / 2.0 + t, 0.0, h / 2.0],
                    Material::Wall,
                ),
                bx(
                    [t, d / 2.0, h / 2.0],
                    [-w / 2.0 - t, 0.0, h / 2.0],
                    Material::Wall,
                ),
                bx(
                    [w / 2.0, t, h / 2.0],
                    [0.0, d / 2.0 + t, h / 2.0],
                    Material::Wall,
                ),
                bx(
                    [w / 2.0, t, h / 2.0],
                    [0.0, -d / 2.0 - t, h / 2.0],
                    Material::Wall,
                ),
            ],
            start: [0.0, 0.0, 0.0],
            start_yaw_deg: 0.0,
            stream_radius: 50.0,
        }
    }

    /// A flat floor of one material, `half` metres each way from the origin.
    pub fn floor(half: f64, material: Material) -> WorldSpec {
        WorldSpec {
            name: "floor".into(),
            colliders: vec![WorldCollider {
                shape: Shape::Box {
                    half: [half, half, 0.1],
                },
                position: [0.0, 0.0, -0.1],
                yaw_deg: 0.0,
                material,
            }],
            start: [0.0; 3],
            start_yaw_deg: 0.0,
            stream_radius: half * 2.0,
        }
    }
}

/// What touched what in the last step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ContactState {
    pub touching: bool,
    /// Open props: which rotors struck something.
    pub strikes: [bool; 4],
}

/// The Rapier side: one dynamic body (the quad) and the live static colliders.
pub struct Physics {
    pub world: PhysicsWorld,
    pub body: RigidBodyHandle,
    hull: Vec<ColliderHandle>,
    rotor_sensors: Vec<ColliderHandle>,
    live: Vec<Option<ColliderHandle>>,
    spec: WorldSpec,
    bottom: f64,
}

impl Physics {
    pub fn new(p: &Params, spec: WorldSpec, dt: f64) -> Physics {
        let mut world = PhysicsWorld::new();
        world.gravity = DVec3::new(0.0, 0.0, -crate::profile::G);
        world.integration_parameters.dt = dt;
        // Small bodies: scale the solver's length tolerances to a whoop, not a metre.
        world.integration_parameters.length_unit = (p.body_half[0] * 2.0).clamp(0.02, 1.0);
        let props = MassProperties::new(
            DVec3::ZERO,
            p.mass,
            DVec3::new(p.inertia[0], p.inertia[1], p.inertia[2]),
        );
        let rb = RigidBodyBuilder::dynamic()
            .additional_mass_properties(props)
            .can_sleep(false)
            .ccd_enabled(true)
            .gyroscopic_forces_enabled(true)
            .build();
        let body = world.insert_body(rb);
        let (rest, fric) = (0.4, 0.5);
        let mut hull = Vec::new();
        let mut rotor_sensors = Vec::new();
        let body_box = ColliderBuilder::cuboid(p.body_half[0], p.body_half[1], p.body_half[2])
            .density(0.0)
            .restitution(rest)
            .friction(fric)
            .build();
        hull.push(world.insert_collider(body_box, Some(body)));
        // Cylinders are along y in Rapier: turn them upright.
        let upright = DVec3::new(std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        for r in &p.rotor_pos {
            let at = DVec3::new(r[0], r[1], r[2]);
            if p.ducted {
                let duct = ColliderBuilder::cylinder(p.duct_height / 2.0, p.duct_radius)
                    .translation(at)
                    .rotation(upright)
                    .density(0.0)
                    .restitution(rest)
                    .friction(fric)
                    .build();
                hull.push(world.insert_collider(duct, Some(body)));
            } else {
                let disc = ColliderBuilder::cylinder(0.005, p.prop_radius)
                    .translation(at)
                    .rotation(upright)
                    .density(0.0)
                    .sensor(true)
                    .build();
                rotor_sensors.push(world.insert_collider(disc, Some(body)));
            }
        }
        let mut ph = Physics {
            world,
            body,
            hull,
            rotor_sensors,
            live: vec![None; spec.colliders.len()],
            spec,
            bottom: Physics::hull_bottom(p),
        };
        ph.place_at_start();
        ph.stream();
        ph
    }

    /// How far the hull reaches below the body origin (m).
    pub fn hull_bottom(p: &Params) -> f64 {
        let duct = if p.ducted { p.duct_height / 2.0 } else { 0.0 };
        p.body_half[2].max(duct)
    }

    pub fn place_at_start(&mut self) {
        let s = self.spec.start;
        let lift = self.bottom;
        let rot = DQuat::from_rotation_z(self.spec.start_yaw_deg.to_radians());
        let rb = &mut self.world.bodies[self.body];
        rb.set_translation(DVec3::new(s[0], s[1], s[2] + lift + 1e-3), true);
        rb.set_rotation(rot, true);
        rb.set_linvel(DVec3::ZERO, true);
        rb.set_angvel(DVec3::ZERO, true);
        self.world.bodies[self.body].reset_forces(true);
        self.world.bodies[self.body].reset_torques(true);
    }

    /// Insert the colliders within the stream radius and remove those past 1.25 × it.
    pub fn stream(&mut self) {
        let at = self.world.bodies[self.body].translation();
        let r_in = self.spec.stream_radius;
        let mut changed = false;
        for i in 0..self.spec.colliders.len() {
            let c = &self.spec.colliders[i];
            let d = (DVec3::from(c.position) - at).length() - c.radius();
            match self.live[i] {
                None if d <= r_in => {
                    let h = self.world.insert_collider(c.build(), None);
                    self.live[i] = Some(h);
                    changed = true;
                }
                Some(h) if d > 1.25 * r_in => {
                    self.world.remove_collider(h);
                    self.live[i] = None;
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            // Bring the query structures up to date now, so ground-effect rays see a new
            // collider before the next solver step.
            self.world.detect_collisions(&(), &());
        }
    }

    pub fn live_colliders(&self) -> usize {
        self.live.iter().filter(|x| x.is_some()).count()
    }

    /// Distance from `origin` along `dir` to the nearest static surface, up to `max`.
    pub fn ray(&self, origin: DVec3, dir: DVec3, max: f64) -> Option<f64> {
        let ray = Ray::new(origin, dir);
        let filter = QueryFilter::default()
            .exclude_rigid_body(self.body)
            .exclude_sensors();
        self.world
            .cast_ray(&ray, max, true, filter)
            .map(|(_, toi)| toi)
    }

    pub fn contacts(&self) -> ContactState {
        let mut s = ContactState::default();
        for h in &self.hull {
            if self
                .world
                .narrow_phase
                .contact_pairs_with(*h)
                .any(|c| c.has_any_active_contact())
            {
                s.touching = true;
            }
        }
        for (i, h) in self.rotor_sensors.iter().enumerate() {
            if self
                .world
                .narrow_phase
                .intersection_pairs_with(*h)
                .any(|(_, _, hit)| hit)
            {
                s.strikes[i] = true;
            }
        }
        s
    }
}
