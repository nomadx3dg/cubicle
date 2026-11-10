//! cubicle: a tiny CPU 3D rasteriser that draws to the terminal as ASCII.
//!
//! Module map (see `handout.md` for the full tour):
//! - [`vec3`]    vector maths
//! - [`mesh`]    triangle meshes, normalisation, smooth normals
//! - [`obj`]     Wavefront OBJ loader
//! - [`raster`]  projection, fill, depth buffer, lighting
//! - [`display`] canvas -> terminal text (ASCII / colour / half-blocks), HUD
//! - [`image`]   PNG loading and image -> canvas
//! - [`cli`]     argument parsing
//! - [`settings`] user settings (backend, render options) and their file
//! - [`app`]     terminal UI loops and `--once` rendering

pub mod app;
pub mod browser;
pub mod cli;
pub mod display;
pub mod engine;
pub mod gpu;
pub mod image;
pub mod menu;
pub mod mesh;
pub mod obj;
pub mod raster;
pub mod settings;
pub mod vec3;
