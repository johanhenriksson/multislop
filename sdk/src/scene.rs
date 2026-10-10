//! Builds the commands the host turns into a scene and HUD: hands out ids, shares identical meshes and materials,
//! and only resends HUD nodes that changed.

use glam::Vec3;
use crate::client::{Color, Command, Material, MeshData, NodeKind, Transform, UiNode};
use std::collections::HashMap;

#[derive(Default)]
pub struct Scene {
    /// Ids for assets, scene nodes and HUD nodes alike; 0 is the camera.
    next: u32,
    pub out: Vec<Command>,
    shared: HashMap<String, u32>,
    /// Every HUD node as last sent.
    ui: HashMap<u32, UiNode>,
}

impl Scene {
    pub fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    pub fn mesh(&mut self, mesh: MeshData) -> u32 {
        let id = self.id();
        self.out.push(Command::Mesh { id, mesh });
        id
    }

    pub fn material(&mut self, material: Material) -> u32 {
        let id = self.id();
        self.out.push(Command::Material { id, material });
        id
    }

    /// One mesh per `key`, made the first time it's asked for.
    pub fn shared_mesh(&mut self, key: String, mesh: impl FnOnce() -> MeshData) -> u32 {
        match self.shared.get(&key) {
            Some(&id) => id,
            None => {
                let id = self.mesh(mesh());
                self.shared.insert(key, id);
                id
            }
        }
    }

    pub fn shared_material(&mut self, key: String, material: impl FnOnce() -> Material) -> u32 {
        match self.shared.get(&key) {
            Some(&id) => id,
            None => {
                let id = self.material(material());
                self.shared.insert(key, id);
                id
            }
        }
    }

    pub fn cuboid(&mut self, size: Vec3) -> u32 {
        self.shared_mesh(format!("cuboid {size}"), || MeshData::Cuboid(size))
    }

    /// A plain material of this color.
    pub fn color(&mut self, color: Color) -> u32 {
        self.shared_material(format!("color {:?}", color.0), || Material::color(color))
    }

    pub fn spawn(&mut self, parent: Option<u32>, kind: NodeKind, transform: Transform, visible: bool) -> u32 {
        let id = self.id();
        self.out.push(Command::Spawn { id, parent, kind, transform, visible });
        id
    }

    pub fn empty(&mut self, parent: Option<u32>, transform: Transform, visible: bool) -> u32 {
        self.spawn(parent, NodeKind::Empty, transform, visible)
    }

    pub fn mesh_node(&mut self, parent: Option<u32>, mesh: u32, material: u32, transform: Transform) -> u32 {
        self.spawn(parent, NodeKind::Mesh { mesh, material, shadows: true }, transform, true)
    }

    pub fn push(&mut self, cmd: Command) {
        self.out.push(cmd);
    }

    /// A new HUD node.
    pub fn ui(&mut self, parent: Option<u32>, node: UiNode) -> u32 {
        let id = self.id();
        self.out.push(Command::Ui { id, parent, node: node.clone() });
        self.ui.insert(id, node);
        id
    }

    /// Changes a HUD node, sending it only if that made a difference.
    pub fn ui_set(&mut self, id: u32, f: impl FnOnce(&mut UiNode)) {
        let Some(old) = self.ui.get(&id) else { return };
        let mut node = old.clone();
        f(&mut node);
        if node != *old {
            self.out.push(Command::Ui { id, parent: None, node: node.clone() });
            self.ui.insert(id, node);
        }
    }

    pub fn ui_text(&mut self, id: u32, value: impl Into<String>) {
        let value = value.into();
        self.ui_set(id, |n| {
            if let Some(t) = &mut n.text {
                t.value = value;
            }
        });
    }

    pub fn ui_hidden(&mut self, id: u32, hidden: bool) {
        self.ui_set(id, |n| n.hidden = hidden);
    }

    /// Removes a HUD node; its children must be removed with `forget` as the host drops them along with it.
    pub fn ui_despawn(&mut self, id: u32) {
        self.ui.remove(&id);
        self.out.push(Command::UiDespawn { id });
    }

    pub fn forget(&mut self, id: u32) {
        self.ui.remove(&id);
    }
}
