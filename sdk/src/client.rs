//! The client half of a game. The host owns the window, netcode, audio and rendering; the game owns everything you
//! see and hear, and describes it with retained `Command`s (assets, a scene tree, a HUD) plus per-frame camera,
//! sounds and lines. Ids are picked by the game; scene node 0 is the camera.

use crate::{Entity, Interpolated};
use glam::{EulerRot, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Scene node id of the camera, as a parent for things that move with it (a first-person weapon).
pub const CAMERA: u32 = 0;

pub trait ClientGame {
    /// First call: assets, the world and the HUD.
    fn start(&mut self) -> Vec<Command>;
    /// A snapshot the netcode accepted (newer than any before).
    fn snapshot(&mut self, snap: &ClientSnapshot);
    /// Once per tick: the input to send, from the keys held now.
    fn input(&mut self, raw: &RawInput) -> Vec<u8>;
    /// The host's prediction step for our own entity. `fresh` is false when replaying after a snapshot, so effects
    /// (sounds, tracers) are only started once.
    fn predict(&mut self, me: &mut Entity, input: &[u8], seq: u32, fresh: bool);
    fn frame(&mut self, f: &FrameIn) -> FrameOut;
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct ClientSnapshot {
    pub tick: u32,
    /// Our entity id.
    pub you: u32,
    pub state: Vec<u8>,
    pub entities: Vec<Entity>,
    pub events: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct FrameIn {
    pub dt: f32,
    /// Seconds since the client started.
    pub time: f64,
    /// Our entity as predicted, and where to draw it (smoothed and in between ticks).
    pub me: Option<Entity>,
    pub me_pos: Vec3,
    /// Everyone else, as the netcode would draw them now.
    pub others: Vec<Interpolated>,
    pub input: RawInput,
    /// A clickable UI node clicked this frame.
    pub clicked: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct FrameOut {
    pub commands: Vec<Command>,
    pub sounds: Vec<Play>,
    pub camera: Camera,
    /// Drawn this frame only.
    pub lines: Vec<(Vec3, Vec3, Color)>,
    pub spheres: Vec<(Vec3, f32, Color)>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct RawInput {
    pub held: Vec<Key>,
    /// Went down since the last frame.
    pub pressed: Vec<Key>,
    pub mouse: Vec2,
    /// The mouse is captured by the game (the host captures on click and releases on Esc).
    pub captured: bool,
}

impl RawInput {
    pub fn held(&self, k: Key) -> bool {
        self.held.contains(&k)
    }

    pub fn pressed(&self, k: Key) -> bool {
        self.pressed.contains(&k)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[rustfmt::skip]
pub enum Key {
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    Space, Enter, Escape, Tab, Backspace, ShiftLeft, ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight,
    Up, Down, Left, Right,
    MouseLeft, MouseRight, MouseMiddle,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub pos: Vec3,
    pub rot: Quat,
    /// Vertical field of view, radians.
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera { pos: Vec3::ZERO, rot: Quat::IDENTITY, fov: 1.6 }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub sound: u32,
    pub volume: f32,
    pub looped: bool,
}

/// sRGB with alpha.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const WHITE: Color = Color([1.0; 4]);
    pub const BLACK: Color = Color([0.0, 0.0, 0.0, 1.0]);
    pub const NONE: Color = Color([0.0; 4]);

    pub const fn srgb(r: f32, g: f32, b: f32) -> Color {
        Color([r, g, b, 1.0])
    }

    pub const fn srgba(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color([r, g, b, a])
    }

    pub const fn with_alpha(self, a: f32) -> Color {
        let [r, g, b, _] = self.0;
        Color([r, g, b, a])
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform { pos: Vec3::ZERO, rot: Quat::IDENTITY, scale: Vec3::ONE }
    }
}

impl Transform {
    pub fn at(pos: Vec3) -> Transform {
        Transform { pos, ..Default::default() }
    }

    pub fn rotated(self, rot: Quat) -> Transform {
        Transform { rot, ..self }
    }

    pub fn scaled(self, scale: f32) -> Transform {
        Transform { scale: Vec3::splat(scale), ..self }
    }
}

/// Rotation for a view with `yaw` and `pitch`, looking down -Z when both are 0.
pub fn look_rot(yaw: f32, pitch: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Command {
    /// RGBA8 sRGB, `mips` levels one after another (each half the size), repeating and smoothly filtered.
    Texture { id: u32, w: u32, h: u32, mips: u32, rgba: Vec<u8> },
    Mesh { id: u32, mesh: MeshData },
    /// Creates a material, or changes it in place for everything using it.
    Material { id: u32, material: Material },
    Sound { id: u32, wav: Vec<u8> },
    /// Drops an asset (texture, mesh, material or sound) nothing uses any more.
    Free { id: u32 },
    Environment(Environment),
    Spawn { id: u32, parent: Option<u32>, kind: NodeKind, transform: Transform, visible: bool },
    /// Changes a scene node; `light` is a point light's intensity.
    Set { id: u32, transform: Option<Transform>, visible: Option<bool>, light: Option<f32> },
    /// Removes a scene node and its children.
    Despawn { id: u32 },
    /// Creates a HUD node, or replaces an existing one's look (it keeps its place in the tree).
    Ui { id: u32, parent: Option<u32>, node: UiNode },
    /// Removes a HUD node and its children.
    UiDespawn { id: u32 },
}

impl Command {
    pub fn set_transform(id: u32, t: Transform) -> Command {
        Command::Set { id, transform: Some(t), visible: None, light: None }
    }

    pub fn set_visible(id: u32, visible: bool) -> Command {
        Command::Set { id, transform: None, visible: Some(visible), light: None }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MeshData {
    Cuboid(Vec3),
    /// UV sphere, or an icosphere with `ico` subdivisions.
    Sphere { radius: f32, ico: Option<u32> },
    /// Along Y.
    Cylinder { radius: f32, height: f32 },
    /// Flat on XZ, facing up.
    Plane(Vec2),
    /// Triangles. Empty normals are computed (smooth); colors are linear and multiply the material.
    Custom { positions: Vec<[f32; 3]>, normals: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, colors: Vec<[f32; 4]>, indices: Vec<u32> },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Blend {
    Opaque,
    /// Cut out below this alpha.
    Mask(f32),
    Blend,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub color: Color,
    pub texture: Option<u32>,
    /// Texture repeats per UV unit.
    pub uv_scale: f32,
    /// Linear RGB.
    pub emissive: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: bool,
    pub blend: Blend,
    pub fog: bool,
    /// Drawn from both sides.
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Material {
        Material {
            color: Color::WHITE,
            texture: None,
            uv_scale: 1.0,
            emissive: [0.0; 3],
            roughness: 0.5,
            metallic: 0.0,
            unlit: false,
            blend: Blend::Opaque,
            fog: true,
            double_sided: false,
        }
    }
}

impl Material {
    pub fn color(color: Color) -> Material {
        Material { color, ..Default::default() }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum NodeKind {
    Empty,
    Mesh { mesh: u32, material: u32, shadows: bool },
    PointLight { color: Color, intensity: f32, range: f32 },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    pub sky: Color,
    pub ambient: Color,
    pub ambient_brightness: f32,
    pub sun_dir: Vec3,
    pub sun_color: Color,
    pub sun_illuminance: f32,
    pub fog_color: Color,
    /// Fog lit by the sun when looking towards it.
    pub fog_sun_color: Color,
    pub fog_sun_exponent: f32,
    pub fog_start: f32,
    pub fog_end: f32,
}

/// A HUD node: a box laid out like CSS flexbox, optionally with text or an image.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UiNode {
    /// Positioned by left/right/top/bottom against its parent instead of flowing.
    pub absolute: bool,
    pub left: Val,
    pub right: Val,
    pub top: Val,
    pub bottom: Val,
    pub width: Val,
    pub height: Val,
    pub margin: Edges,
    pub padding: Edges,
    pub border: Edges,
    /// Children flow top to bottom instead of left to right.
    pub column: bool,
    /// Along the flow, and across it.
    pub justify: Align,
    pub align: Align,
    pub gap: f32,
    pub background: Color,
    pub border_color: Color,
    pub radius: Corners,
    pub outline: Option<(f32, Color)>,
    pub text: Option<Text>,
    pub image: Option<Image>,
    pub hidden: bool,
    /// Reported in `FrameIn::clicked` when clicked.
    pub clickable: bool,
    /// Kept centered on where this world point is on screen (hidden when behind the camera).
    pub world: Option<Vec3>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Val {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
    /// Percent of the window height.
    Vh(f32),
}

pub const fn px(v: f32) -> Val {
    Val::Px(v)
}

pub const fn percent(v: f32) -> Val {
    Val::Percent(v)
}

pub const fn vh(v: f32) -> Val {
    Val::Vh(v)
}

/// Pixels per side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Edges {
    pub const fn all(v: f32) -> Edges {
        Edges { left: v, right: v, top: v, bottom: v }
    }

    pub const fn axes(h: f32, v: f32) -> Edges {
        Edges { left: h, right: h, top: v, bottom: v }
    }

    pub const fn left(v: f32) -> Edges {
        Edges { left: v, right: 0.0, top: 0.0, bottom: 0.0 }
    }

    pub const fn top(v: f32) -> Edges {
        Edges { left: 0.0, right: 0.0, top: v, bottom: 0.0 }
    }
}

/// Corner radii in pixels; a huge radius makes a circle.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub const fn all(v: f32) -> Corners {
        Corners { top_left: v, top_right: v, bottom_right: v, bottom_left: v }
    }

    pub const fn bottom(v: f32) -> Corners {
        Corners { top_left: 0.0, top_right: 0.0, bottom_right: v, bottom_left: v }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Align {
    /// The flexbox default: start along the flow, stretch across it.
    #[default]
    Default,
    Start,
    Center,
    End,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Text {
    pub value: String,
    pub size: f32,
    pub color: Color,
    pub shadow: bool,
    /// Lines centered rather than left aligned.
    pub center: bool,
}

impl Text {
    pub fn new(value: impl Into<String>, size: f32, color: Color) -> Text {
        Text { value: value.into(), size, color, shadow: false, center: false }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub texture: u32,
    pub tint: Color,
}

/// Exports a `ClientGame` from a wasm module, made by `$new` on first use. Every export takes its argument from the
/// shared buffer and returns `ptr << 32 | len` of its (possibly empty) result there.
#[macro_export]
macro_rules! export_client {
    ($new:expr) => {
        #[cfg(target_arch = "wasm32")]
        mod fps16_client_exports {
            use super::*;
            use std::cell::RefCell;
            use $crate::client::{ClientGame, ClientSnapshot, FrameIn, RawInput};
            use $crate::{Entity, decode, encode};

            thread_local! {
                static CLIENT: RefCell<Box<dyn ClientGame>> = RefCell::new({
                    std::panic::set_hook(Box::new(|info| $crate::log(&info.to_string())));
                    Box::new($new)
                });
            }

            fn client<R>(f: impl FnOnce(&mut dyn ClientGame) -> R) -> R {
                CLIENT.with_borrow_mut(|c| f(c.as_mut()))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_client_start(_len: usize) -> u64 {
                $crate::wasm_ret(encode(&client(|c| c.start())))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_client_snapshot(len: usize) -> u64 {
                let snap: ClientSnapshot = $crate::wasm_arg(len, |b| decode(b)).unwrap_or_default();
                client(|c| c.snapshot(&snap));
                $crate::wasm_ret(Vec::new())
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_client_input(len: usize) -> u64 {
                let raw: RawInput = $crate::wasm_arg(len, |b| decode(b)).unwrap_or_default();
                $crate::wasm_ret(client(|c| c.input(&raw)))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_client_predict(len: usize) -> u64 {
                let (mut me, input, seq, fresh): (Entity, Vec<u8>, u32, bool) =
                    $crate::wasm_arg(len, |b| decode(b)).unwrap_or_default();
                client(|c| c.predict(&mut me, &input, seq, fresh));
                $crate::wasm_ret(encode(&me))
            }

            #[unsafe(no_mangle)]
            extern "C" fn fps16_client_frame(len: usize) -> u64 {
                let f: FrameIn = $crate::wasm_arg(len, |b| decode(b)).unwrap_or_default();
                $crate::wasm_ret(encode(&client(|c| c.frame(&f))))
            }
        }
    };
}
