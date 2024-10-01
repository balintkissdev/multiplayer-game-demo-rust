use std::{collections::HashMap, sync::Arc};

use cgmath::{Matrix, Matrix4, Vector2, Vector3};
use glow::HasContext;
use glutin::{
    config::{ConfigTemplateBuilder, GlConfig},
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version},
    display::GetGlDisplay,
    prelude::{GlDisplay, NotCurrentGlContext},
    surface::{GlSurface, Surface, WindowSurface},
};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::{
    dpi::PhysicalSize,
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes},
};

use crate::{fsm, globals, gui::Gui, Player, PlayerID};

const GRID_COL_COUNT: usize = 40;
const GRID_ROW_COUNT: usize = GRID_COL_COUNT;

const GRID_VERTEX_SHADER_SRC: &str = r#"
    #version 120

    attribute vec2 aPos;
    uniform mat4 uMVP;

    void main() {
        gl_Position = uMVP * vec4(aPos, 0.0, 1.0);
    }
"#;

const GRID_FRAGMENT_SHADER_SRC: &str = r#"
    #version 120

    void main() {
        gl_FragColor = vec4(0.5, 0.5, 0.5, 1.0);
    }
"#;

const QUAD_VERTEX_SHADER_SRC: &str = r#"
    #version 120

    attribute vec2 aPos;
    uniform mat4 uMVP;

    void main() {
        gl_Position = uMVP * vec4(aPos.x, aPos.y, 0.0, 1.0);
    }
"#;

const QUAD_FRAGMENT_SHADER_SRC: &str = r#"
    #version 120

    uniform vec3 uColor;

    void main() {
        gl_FragColor = vec4(uColor, 1.0);
    }
"#;

pub struct Renderer {
    // There's no VAO for OpenGL 2.1
    grid_shader_program: glow::Program,
    grid_vbo: glow::Buffer,
    grid_mvp_location: glow::UniformLocation,
    quad_mvp_location: glow::UniformLocation,
    quad_color_location: glow::UniformLocation,
    quad_shader_program: glow::Program,
    quad_vbo: glow::Buffer,
    gl_surface: Surface<WindowSurface>,
    gl_context: PossiblyCurrentContext,
    gl: Arc<glow::Context>,
}

impl Renderer {
    pub fn create_graphics(event_loop: &ActiveEventLoop) -> (Window, Renderer, Gui) {
        unsafe {
            let window_attributes = WindowAttributes::default()
                .with_title(globals::WINDOW_TITLE)
                .with_inner_size(PhysicalSize::new(
                    globals::WINDOW_SIZE.0,
                    globals::WINDOW_SIZE.1,
                ))
                .with_resizable(false);
            let display_builder =
                DisplayBuilder::new().with_window_attributes(Some(window_attributes));
            let (window, gl_config) = display_builder
                .build(event_loop, ConfigTemplateBuilder::new(), |configs| {
                    configs
                        .reduce(|accum, config| {
                            if config.num_samples() > accum.num_samples() {
                                config
                            } else {
                                accum
                            }
                        })
                        .unwrap()
                })
                .unwrap();

            let raw_window_handle = window
                .as_ref()
                .and_then(|w| w.window_handle().ok())
                .map(|h| h.as_raw());

            let gl_display = gl_config.display();
            let context_attributes = ContextAttributesBuilder::new()
                .with_context_api(ContextApi::OpenGl(Some(Version { major: 2, minor: 1 })))
                .build(raw_window_handle);
            let not_current_gl_context = gl_display
                .create_context(&gl_config, &context_attributes)
                .unwrap();

            let window = window.unwrap();

            let surface_attributes = window.build_surface_attributes(Default::default()).unwrap();
            let gl_surface = gl_display
                .create_window_surface(&gl_config, &surface_attributes)
                .unwrap();
            let gl_context = not_current_gl_context.make_current(&gl_surface).unwrap();

            let gl = glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s));

            gl.clear_color(1.0, 1.0, 1.0, 1.0);

            let quad_vertex_shader = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(quad_vertex_shader, QUAD_VERTEX_SHADER_SRC);
            gl.compile_shader(quad_vertex_shader);

            let quad_fragment_shader = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(quad_fragment_shader, QUAD_FRAGMENT_SHADER_SRC);
            gl.compile_shader(quad_fragment_shader);

            let quad_shader_program = gl.create_program().unwrap();
            gl.attach_shader(quad_shader_program, quad_vertex_shader);
            gl.attach_shader(quad_shader_program, quad_fragment_shader);
            gl.link_program(quad_shader_program);
            gl.use_program(Some(quad_shader_program));

            gl.delete_shader(quad_vertex_shader);
            gl.delete_shader(quad_fragment_shader);

            let quad_vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad_vbo));

            let quad_vertices: [f32; 12] =
                [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0];

            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&quad_vertices),
                glow::STATIC_DRAW,
            );

            let quad_mvp_location = gl
                .get_uniform_location(quad_shader_program, "uMVP")
                .unwrap();
            let quad_color_location = gl
                .get_uniform_location(quad_shader_program, "uColor")
                .unwrap();

            gl.use_program(None);

            let grid_vertex_shader = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(grid_vertex_shader, GRID_VERTEX_SHADER_SRC);
            gl.compile_shader(grid_vertex_shader);

            let grid_fragment_shader = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(grid_fragment_shader, GRID_FRAGMENT_SHADER_SRC);
            gl.compile_shader(grid_fragment_shader);

            let grid_shader_program = gl.create_program().unwrap();
            gl.attach_shader(grid_shader_program, grid_vertex_shader);
            gl.attach_shader(grid_shader_program, grid_fragment_shader);
            gl.link_program(grid_shader_program);
            gl.use_program(Some(grid_shader_program));

            gl.delete_shader(grid_vertex_shader);
            gl.delete_shader(grid_fragment_shader);

            let grid_vertices: Vec<f32> = create_grid_vertices(
                GRID_COL_COUNT,
                GRID_ROW_COUNT,
                globals::WORLD_BOUNDS.max_x * 2.0,
                globals::WORLD_BOUNDS.max_y * 2.0,
            );
            let grid_vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(grid_vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&grid_vertices),
                glow::STATIC_DRAW,
            );

            let grid_mvp_location = gl
                .get_uniform_location(grid_shader_program, "uMVP")
                .unwrap();

            gl.use_program(None);

            let gl = Arc::new(gl);

            let renderer = Self {
                gl: gl.clone(),
                gl_context,
                gl_surface,
                grid_shader_program,
                grid_vbo,
                grid_mvp_location,
                quad_shader_program,
                quad_vbo,
                quad_mvp_location,
                quad_color_location,
            };

            let gui = Gui::new(&event_loop, gl.clone());

            (window, renderer, gui)
        }
    }

    // TODO: Ideally rendering should not know about game logic
    // TODO: Occlusion culling based on camera area
    // TODO: Batch draw calls
    pub fn draw(
        &self,
        camera: &Vector2<f32>,
        local_player: &Player,
        remote_players: &HashMap<PlayerID, Player>,
        state: Option<&fsm::State>,
    ) {
        unsafe {
            self.gl.clear(glow::COLOR_BUFFER_BIT);

            let projection: Matrix4<f32> = cgmath::ortho(
                0.0,
                globals::WINDOW_SIZE.0 as f32,
                globals::WINDOW_SIZE.1 as f32,
                0.0,
                -1.0,
                1.0,
            );
            let camera_offset = Vector2::new(
                globals::WINDOW_SIZE.0 as f32 / 2.0,
                globals::WINDOW_SIZE.1 as f32 / 2.0,
            );
            let view = Matrix4::from_translation(Vector3::new(
                -camera.x + camera_offset.x,
                -camera.y + camera_offset.y,
                0.0,
            ));
            let pv = projection * view;

            self.draw_grid(&pv);

            if matches!(
                state,
                Some(fsm::State::Playing) | Some(fsm::State::QuitDialog)
            ) {
                self.draw_quads(local_player, remote_players, &pv);
            }
        }
    }

    pub fn swap_buffers(&self) {
        self.gl_surface.swap_buffers(&self.gl_context).unwrap();
    }

    fn draw_grid(&self, pv: &Matrix4<f32>) {
        unsafe {
            self.gl.use_program(Some(self.grid_shader_program));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.grid_vbo));

            let grid_position_attrib_location = self
                .gl
                .get_attrib_location(self.grid_shader_program, "aPos")
                .unwrap();
            self.gl
                .enable_vertex_attrib_array(grid_position_attrib_location);
            self.gl.vertex_attrib_pointer_f32(
                grid_position_attrib_location,
                2,
                glow::FLOAT,
                false,
                8,
                0,
            );

            let translation = Matrix4::from_translation(cgmath::vec3(
                globals::WORLD_BOUNDS.min_x,
                globals::WORLD_BOUNDS.min_y,
                0.0,
            ));
            let model = translation;
            let mvp = pv * model;

            let mvp_slice = std::slice::from_raw_parts(mvp.as_ptr(), 16);
            self.gl
                .uniform_matrix_4_f32_slice(Some(&self.grid_mvp_location), false, mvp_slice);
            self.gl.draw_arrays(
                glow::LINES,
                0,
                ((GRID_COL_COUNT + 1) * 2 + (GRID_COL_COUNT + 1) * 2) as i32,
            );
        }
    }

    fn draw_quads(
        &self,
        local_player: &Player,
        remote_players: &HashMap<PlayerID, Player>,
        pv: &Matrix4<f32>,
    ) {
        unsafe {
            self.gl.use_program(Some(self.quad_shader_program));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.quad_vbo));

            let quad_position_attrib_location = self
                .gl
                .get_attrib_location(self.quad_shader_program, "aPos")
                .unwrap();
            self.gl
                .enable_vertex_attrib_array(quad_position_attrib_location);
            self.gl.vertex_attrib_pointer_f32(
                quad_position_attrib_location,
                2,
                glow::FLOAT,
                false,
                8,
                0,
            );

            self.draw_quad(&local_player.pos, &local_player.color, &pv);
            for (_, p) in remote_players.iter() {
                self.draw_quad(&p.pos, &p.color, &pv);
            }
        }
    }

    fn draw_quad(&self, pos: &Vector2<f32>, color: &Vector3<f32>, pv: &Matrix4<f32>) {
        // Move to position
        let mut model = Matrix4::from_translation(cgmath::vec3(pos.x, pos.y, 0.0));
        // Move origin from bottom-right corner to center
        model = model
            * Matrix4::from_translation(cgmath::vec3(
                -0.5 * globals::PLAYER_QUAD_SIZE,
                -0.5 * globals::PLAYER_QUAD_SIZE,
                0.0,
            ));
        // Scale
        model = model * Matrix4::from_scale(globals::PLAYER_QUAD_SIZE);
        let mvp = pv * model;

        unsafe {
            let mvp_slice = std::slice::from_raw_parts(mvp.as_ptr(), 16);
            self.gl
                .uniform_matrix_4_f32_slice(Some(&self.quad_mvp_location), false, mvp_slice);

            self.gl.uniform_3_f32(
                Some(&self.quad_color_location),
                color[0],
                color[1],
                color[2],
            );

            self.gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.quad_shader_program);
            self.gl.delete_buffer(self.quad_vbo);
            self.gl.delete_program(self.grid_shader_program);
            self.gl.delete_buffer(self.grid_vbo);
        }
    }
}

fn create_grid_vertices(
    col_count: usize,
    row_count: usize,
    area_width: f32,
    area_height: f32,
) -> Vec<f32> {
    let mut vertices = Vec::new();
    let cell_width = area_width / col_count as f32;
    let cell_height = area_height / row_count as f32;

    // Vertical lines
    for i in 0..=col_count {
        let x = i as f32 * cell_width;
        vertices.extend_from_slice(&[x, 0.0, x, area_height]);
    }

    // Horizontal lines
    for i in 0..=row_count {
        let y = i as f32 * cell_height;
        vertices.extend_from_slice(&[0.0, y, area_width, y]);
    }

    vertices
}
