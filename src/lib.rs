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
//! - [`app`]     terminal UI loops and `--once` rendering

pub mod app;
pub mod cli;
pub mod display;
pub mod image;
pub mod mesh;
pub mod obj;
pub mod raster;
pub mod vec3;
