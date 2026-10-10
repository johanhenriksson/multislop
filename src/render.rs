//! Turns a game's `sdk::client::Command`s into Bevy assets, entities and UI, keyed by the game's ids.

use bevy::asset::RenderAssetUsages;
use bevy::audio::Volume;
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sdk::client as g;
use std::collections::HashMap;

enum Asset {
    Mesh(Handle<Mesh>),
    Texture(Handle<Image>),
    Material(Handle<StandardMaterial>),
    Sound(Handle<AudioSource>),
}

/// The game's ids and what they became.
#[derive(Resource)]
pub struct Ids {
    pub camera: Entity,
    assets: HashMap<u32, Asset>,
    nodes: HashMap<u32, Entity>,
    ui: HashMap<u32, Entity>,
    /// Point light color and range per node, to change just the intensity.
    lights: HashMap<u32, (Color, f32)>,
    sun: Option<Entity>,
    /// HUD nodes that report clicks.
    clickable: std::collections::HashSet<u32>,
}

impl Ids {
    pub fn new(camera: Entity) -> Ids {
        Ids { camera, assets: HashMap::new(), nodes: HashMap::new(), ui: HashMap::new(), lights: HashMap::new(), sun: None, clickable: default() }
    }
}

/// The clickable HUD node clicked since the last frame.
#[derive(Resource, Default)]
pub struct Clicked(pub Option<u32>);

/// A sound the game started, stopped when the game is torn down.
#[derive(Component)]
pub struct GameSound;

/// Keeps a HUD node centered on a world point.
#[derive(Component)]
pub struct WorldAnchor(Vec3);

#[derive(SystemParam)]
pub struct Renderer<'w, 's> {
    commands: Commands<'w, 's>,
    ids: ResMut<'w, Ids>,
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    mats: ResMut<'w, Assets<StandardMaterial>>,
    audio: ResMut<'w, Assets<AudioSource>>,
    playing: Query<'w, 's, Entity, With<GameSound>>,
}

pub fn color(c: g::Color) -> Color {
    let [r, g, b, a] = c.0;
    Color::srgba(r, g, b, a)
}

fn transform(t: g::Transform) -> Transform {
    Transform { translation: t.pos, rotation: t.rot, scale: t.scale }
}

fn visibility(visible: bool) -> Visibility {
    if visible { Visibility::Inherited } else { Visibility::Hidden }
}

impl Renderer<'_, '_> {
    pub fn apply(&mut self, cmds: Vec<g::Command>) {
        for cmd in cmds {
            self.apply_one(cmd);
        }
    }

    pub fn play(&mut self, sounds: &[g::Play]) {
        for p in sounds {
            let Some(Asset::Sound(handle)) = self.ids.assets.get(&p.sound) else { continue };
            let settings = if p.looped { PlaybackSettings::LOOP } else { PlaybackSettings::DESPAWN };
            self.commands.spawn((GameSound, AudioPlayer::new(handle.clone()), PlaybackSettings { volume: Volume::Linear(p.volume), ..settings }));
        }
    }

    /// Removes everything the game made: scene, HUD, sounds, environment and assets.
    pub fn clear(&mut self) {
        let ids = &mut *self.ids;
        let entities = ids.nodes.drain().chain(ids.ui.drain()).map(|(_, e)| e).chain(ids.sun.take()).chain(&self.playing);
        for e in entities.collect::<Vec<_>>() {
            self.commands.entity(e).try_despawn();
        }
        (ids.assets, ids.lights, ids.clickable) = default();
        self.commands.entity(ids.camera).remove::<DistanceFog>();
        self.commands.insert_resource(ClearColor::default());
    }

    fn apply_one(&mut self, cmd: g::Command) {
        match cmd {
            g::Command::Texture { id, w, h, mips, rgba } => {
                let size = Extent3d { width: w, height: h, depth_or_array_layers: 1 };
                let mut image = Image::new(size, TextureDimension::D2, rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
                image.texture_descriptor.mip_level_count = mips;
                image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    mag_filter: ImageFilterMode::Linear,
                    min_filter: ImageFilterMode::Linear,
                    mipmap_filter: ImageFilterMode::Linear,
                    anisotropy_clamp: 8,
                    ..default()
                });
                self.ids.assets.insert(id, Asset::Texture(self.images.add(image)));
            }
            g::Command::Mesh { id, mesh } => {
                let mesh = self.meshes.add(build_mesh(mesh));
                self.ids.assets.insert(id, Asset::Mesh(mesh));
            }
            g::Command::Material { id, material } => {
                let m = self.material(material);
                match self.ids.assets.get(&id) {
                    Some(Asset::Material(handle)) => {
                        if let Some(mut existing) = self.mats.get_mut(handle) {
                            *existing = m;
                        }
                    }
                    _ => {
                        let handle = self.mats.add(m);
                        self.ids.assets.insert(id, Asset::Material(handle));
                    }
                }
            }
            g::Command::Sound { id, wav } => {
                let handle = self.audio.add(AudioSource { bytes: wav.into() });
                self.ids.assets.insert(id, Asset::Sound(handle));
            }
            g::Command::Free { id } => {
                self.ids.assets.remove(&id);
            }
            g::Command::Environment(env) => self.environment(env),
            g::Command::Spawn { id, parent, kind, transform: t, visible } => {
                let mut e = self.commands.spawn((transform(t), visibility(visible)));
                match kind {
                    g::NodeKind::Empty => {}
                    g::NodeKind::Mesh { mesh, material, shadows } => {
                        if let Some(Asset::Mesh(m)) = self.ids.assets.get(&mesh) {
                            e.insert(Mesh3d(m.clone()));
                        }
                        if let Some(Asset::Material(m)) = self.ids.assets.get(&material) {
                            e.insert(MeshMaterial3d(m.clone()));
                        }
                        if !shadows {
                            e.insert(bevy::light::NotShadowCaster);
                        }
                    }
                    g::NodeKind::PointLight { color: c, intensity, range } => {
                        e.insert(PointLight { color: color(c), intensity, range, ..default() });
                        self.ids.lights.insert(id, (color(c), range));
                    }
                }
                let parent = match parent {
                    Some(g::CAMERA) => Some(self.ids.camera),
                    Some(p) => self.ids.nodes.get(&p).copied(),
                    None => None,
                };
                if let Some(p) = parent {
                    e.insert(ChildOf(p));
                }
                let entity = e.id();
                self.ids.nodes.insert(id, entity);
            }
            g::Command::Set { id, transform: t, visible, light } => {
                let Some(&entity) = self.ids.nodes.get(&id) else { return };
                let mut e = self.commands.entity(entity);
                if let Some(t) = t {
                    e.insert(transform(t));
                }
                if let Some(v) = visible {
                    e.insert(visibility(v));
                }
                if let (Some(intensity), Some(&(color, range))) = (light, self.ids.lights.get(&id)) {
                    e.insert(PointLight { color, intensity, range, ..default() });
                }
            }
            g::Command::Despawn { id } => {
                if let Some(entity) = self.ids.nodes.remove(&id) {
                    self.commands.entity(entity).despawn();
                }
                self.ids.lights.remove(&id);
            }
            g::Command::Ui { id, parent, node } => self.ui(id, parent, node),
            g::Command::UiDespawn { id } => {
                if let Some(entity) = self.ids.ui.remove(&id) {
                    self.commands.entity(entity).despawn();
                }
            }
        }
    }

    fn material(&self, m: g::Material) -> StandardMaterial {
        let texture = match m.texture.and_then(|t| self.ids.assets.get(&t)) {
            Some(Asset::Texture(h)) => Some(h.clone()),
            _ => None,
        };
        StandardMaterial {
            base_color: color(m.color),
            base_color_texture: texture,
            uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(m.uv_scale)),
            emissive: LinearRgba::rgb(m.emissive[0], m.emissive[1], m.emissive[2]),
            perceptual_roughness: m.roughness,
            metallic: m.metallic,
            unlit: m.unlit,
            alpha_mode: match m.blend {
                g::Blend::Opaque => AlphaMode::Opaque,
                g::Blend::Mask(cut) => AlphaMode::Mask(cut),
                g::Blend::Blend => AlphaMode::Blend,
            },
            fog_enabled: m.fog,
            cull_mode: if m.double_sided { None } else { Some(bevy::render::render_resource::Face::Back) },
            double_sided: m.double_sided,
            ..default()
        }
    }

    fn environment(&mut self, env: g::Environment) {
        self.commands.insert_resource(ClearColor(color(env.sky)));
        self.commands.insert_resource(GlobalAmbientLight { brightness: env.ambient_brightness, color: color(env.ambient), ..default() });
        let sun = (
            DirectionalLight { illuminance: env.sun_illuminance, color: color(env.sun_color), shadow_maps_enabled: true, ..default() },
            Transform::default().looking_to(env.sun_dir, Vec3::Y),
        );
        match self.ids.sun {
            Some(e) => {
                self.commands.entity(e).insert(sun);
            }
            None => self.ids.sun = Some(self.commands.spawn(sun).id()),
        }
        self.commands.entity(self.ids.camera).insert(DistanceFog {
            color: color(env.fog_color),
            directional_light_color: color(env.fog_sun_color),
            directional_light_exponent: env.fog_sun_exponent,
            falloff: FogFalloff::Linear { start: env.fog_start, end: env.fog_end },
        });
    }

    fn ui(&mut self, id: u32, parent: Option<u32>, n: g::UiNode) {
        let val = |v: g::Val| match v {
            g::Val::Auto => Val::Auto,
            g::Val::Px(p) => Val::Px(p),
            g::Val::Percent(p) => Val::Percent(p),
            g::Val::Vh(p) => Val::Vh(p),
        };
        let rect = |e: g::Edges| UiRect { left: px(e.left), right: px(e.right), top: px(e.top), bottom: px(e.bottom) };
        let justify = match n.justify {
            g::Align::Default => JustifyContent::Default,
            g::Align::Start => JustifyContent::FlexStart,
            g::Align::Center => JustifyContent::Center,
            g::Align::End => JustifyContent::FlexEnd,
        };
        let align = match n.align {
            g::Align::Default => AlignItems::Default,
            g::Align::Start => AlignItems::FlexStart,
            g::Align::Center => AlignItems::Center,
            g::Align::End => AlignItems::FlexEnd,
        };
        let r = n.radius;
        let node = Node {
            position_type: if n.absolute { PositionType::Absolute } else { PositionType::Relative },
            left: val(n.left),
            right: val(n.right),
            top: val(n.top),
            bottom: val(n.bottom),
            width: val(n.width),
            height: val(n.height),
            margin: rect(n.margin),
            padding: rect(n.padding),
            border: rect(n.border),
            flex_direction: if n.column { FlexDirection::Column } else { FlexDirection::Row },
            justify_content: justify,
            align_items: align,
            row_gap: px(n.gap),
            column_gap: px(n.gap),
            border_radius: BorderRadius {
                top_left: px(r.top_left).into(),
                top_right: px(r.top_right).into(),
                bottom_right: px(r.bottom_right).into(),
                bottom_left: px(r.bottom_left).into(),
            },
            ..default()
        };
        let entity = match self.ids.ui.get(&id) {
            Some(&e) => e,
            None => {
                let e = self.commands.spawn(Node::default()).id();
                let parent = parent.and_then(|p| self.ids.ui.get(&p).copied());
                if let Some(p) = parent {
                    self.commands.entity(e).insert(ChildOf(p));
                }
                self.ids.ui.insert(id, e);
                e
            }
        };
        let image = match n.image.and_then(|i| self.ids.assets.get(&i.texture).map(|a| (a, i.tint))) {
            Some((Asset::Texture(h), tint)) => Some(ImageNode::new(h.clone()).with_color(color(tint))),
            _ => None,
        };
        let mut e = self.commands.entity(entity);
        e.remove::<(Text, TextFont, TextColor, TextShadow, TextLayout, Outline, ImageNode, WorldAnchor)>();
        e.insert((node, BackgroundColor(color(n.background)), BorderColor::all(color(n.border_color)), visibility(!n.hidden)));
        if let Some((width, c)) = n.outline {
            e.insert(Outline::new(px(width), Val::ZERO, color(c)));
        }
        if let Some(t) = n.text {
            e.insert((Text::new(t.value), TextFont { font_size: FontSize::Px(t.size), ..default() }, TextColor(color(t.color))));
            if t.shadow {
                e.insert(TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.85) });
            }
            if t.center {
                e.insert(TextLayout { justify: Justify::Center, ..default() });
            }
        }
        if let Some(image) = image {
            e.insert(image);
        }
        if n.clickable && self.ids.clickable.insert(id) {
            e.observe(move |_: On<bevy::picking::events::PointerClick>, mut clicked: ResMut<Clicked>| clicked.0 = Some(id));
        }
        if let Some(at) = n.world {
            e.insert(WorldAnchor(at));
        }
    }
}

fn build_mesh(mesh: g::MeshData) -> Mesh {
    match mesh {
        g::MeshData::Cuboid(size) => Cuboid::from_size(size).into(),
        g::MeshData::Sphere { radius, ico: Some(n) } => Sphere::new(radius).mesh().ico(n).unwrap(),
        g::MeshData::Sphere { radius, ico: None } => Sphere::new(radius).into(),
        g::MeshData::Cylinder { radius, height } => Cylinder::new(radius, height).into(),
        g::MeshData::Plane(size) => Plane3d::default().mesh().size(size.x, size.y).build(),
        g::MeshData::Custom { positions, normals, uvs, colors, indices } => {
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_indices(Indices::U32(indices));
            if !uvs.is_empty() {
                m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            }
            if !colors.is_empty() {
                m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            }
            if normals.is_empty() {
                m.compute_smooth_normals();
            } else {
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            }
            m
        }
    }
}

/// Keeps world-anchored HUD nodes centered on their point, hidden when it's behind the camera.
pub fn anchor_ui(
    camera: Single<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut q: Query<(&WorldAnchor, &mut Node, &ComputedNode, &mut Visibility)>,
) {
    let (cam, cam_t) = *camera;
    for (anchor, mut node, computed, mut vis) in &mut q {
        match cam.world_to_viewport(cam_t, anchor.0) {
            Ok(at) => {
                let size = computed.size() * computed.inverse_scale_factor();
                (node.left, node.top) = (px(at.x - size.x / 2.0), px(at.y - size.y / 2.0));
                *vis = Visibility::Inherited;
            }
            Err(_) => *vis = Visibility::Hidden,
        }
    }
}
