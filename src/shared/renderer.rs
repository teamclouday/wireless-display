use std::{ffi::CString, num::NonZeroU32, sync::Arc};

use anyhow::Result;
use gl::types::*;
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version},
    display::{Display, DisplayApiPreference},
    prelude::{GlDisplay, NotCurrentGlContext},
    surface::{Surface, SurfaceAttributesBuilder, WindowSurface},
};
use winit::{
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::Window,
};

const VERTEX_SHADER_SOURCE: &str = r#"
#version 330 core
layout (location = 0) in vec2 aPos;
layout (location = 1) in vec2 aTexCoord;

out vec2 TexCoord;

void main()
{
    gl_Position = vec4(aPos.x, aPos.y, 0.0, 1.0);
    TexCoord = aTexCoord;
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
#version 330 core
out vec4 FragColor;

in vec2 TexCoord;
uniform sampler2D frameTexture;

void main()
{
    vec4 tex = texture(frameTexture, TexCoord);
    FragColor = vec4((tex.rgb - vec3(16.0/255.0)) * (255.0/219.0), tex.a);
}
"#;

#[derive(Clone)]
pub struct OpenGLRenderer {
    vao: GLuint,
    vbo: GLuint,
    ebo: GLuint,
    texture: GLuint,
    shader: GLuint,
    width: u32,
    height: u32,
}

impl OpenGLRenderer {
    pub fn new() -> Result<Self> {
        unsafe {
            // compile vertex shader
            let vertex_shader = gl::CreateShader(gl::VERTEX_SHADER);
            let c_str_vert = CString::new(VERTEX_SHADER_SOURCE.as_bytes())?;
            gl::ShaderSource(vertex_shader, 1, &c_str_vert.as_ptr(), std::ptr::null());
            gl::CompileShader(vertex_shader);

            // compile fragment shader
            let fragment_shader = gl::CreateShader(gl::FRAGMENT_SHADER);
            let c_str_frag = CString::new(FRAGMENT_SHADER_SOURCE.as_bytes())?;
            gl::ShaderSource(fragment_shader, 1, &c_str_frag.as_ptr(), std::ptr::null());
            gl::CompileShader(fragment_shader);

            // link shaders to create shader program
            let shader_program = gl::CreateProgram();
            gl::AttachShader(shader_program, vertex_shader);
            gl::AttachShader(shader_program, fragment_shader);
            gl::LinkProgram(shader_program);
            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);

            // set up vertex data and buffers
            let vertices: [GLfloat; 16] = [
                // positions   // texture coords
                1.0, 1.0, 1.0, 0.0, // top right
                1.0, -1.0, 1.0, 1.0, // bottom right
                -1.0, -1.0, 0.0, 1.0, // bottom left
                -1.0, 1.0, 0.0, 0.0, // top left
            ];

            let indices: [GLuint; 6] = [
                0, 1, 3, // first triangle
                1, 2, 3, // second triangle
            ];

            let mut vao: GLuint = 0;
            let mut vbo: GLuint = 0;
            let mut ebo: GLuint = 0;

            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);

            gl::BindVertexArray(vao);

            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<GLfloat>()) as GLsizeiptr,
                vertices.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );

            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * std::mem::size_of::<GLuint>()) as GLsizeiptr,
                indices.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );

            // position attribute
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                4 * std::mem::size_of::<GLfloat>() as GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);

            // texture coord attribute
            gl::VertexAttribPointer(
                1,
                2,
                gl::FLOAT,
                gl::FALSE,
                4 * std::mem::size_of::<GLfloat>() as GLsizei,
                (2 * std::mem::size_of::<GLfloat>()) as *const GLvoid,
            );
            gl::EnableVertexAttribArray(1);

            // create texture
            let mut texture: GLuint = 0;
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);

            // set texture parameters
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_WRAP_S,
                gl::CLAMP_TO_EDGE as GLint,
            );
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_WRAP_T,
                gl::CLAMP_TO_EDGE as GLint,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as GLint);

            Ok(Self {
                vao,
                vbo,
                ebo,
                texture,
                shader: shader_program,
                width: 0,
                height: 0,
            })
        }
    }

    pub fn update_texture(&mut self, data: &[u8], width: u32, height: u32) {
        unsafe {
            self.width = width;
            self.height = height;

            gl::BindTexture(gl::TEXTURE_2D, self.texture);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as GLint,
                width as GLsizei,
                height as GLsizei,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const GLvoid,
            );
        }
    }

    pub fn render(&self, width: u32, height: u32) {
        unsafe {
            let frame_aspect = self.width as f32 / self.height as f32;
            let window_aspect = width as f32 / height as f32;

            let (scale_x, scale_y) = if window_aspect > frame_aspect {
                (frame_aspect / window_aspect, 1.0)
            } else {
                (1.0, window_aspect / frame_aspect)
            };

            // update vertex data
            let vertices: [GLfloat; 16] = [
                // positions   // texture coords
                scale_x, scale_y, 1.0, 0.0, // top right
                scale_x, -scale_y, 1.0, 1.0, // bottom right
                -scale_x, -scale_y, 0.0, 1.0, // bottom left
                -scale_x, scale_y, 0.0, 0.0, // top left
            ];

            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferSubData(
                gl::ARRAY_BUFFER,
                0,
                (vertices.len() * std::mem::size_of::<GLfloat>()) as GLsizeiptr,
                vertices.as_ptr() as *const GLvoid,
            );

            // render
            gl::Viewport(0, 0, width as GLsizei, height as GLsizei);
            gl::ClearColor(0.0, 0.0, 0.0, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            gl::UseProgram(self.shader);
            gl::BindVertexArray(self.vao);
            gl::BindTexture(gl::TEXTURE_2D, self.texture);
            gl::DrawElements(
                gl::TRIANGLES,
                6,
                gl::UNSIGNED_INT,
                std::ptr::null() as *const GLvoid,
            );
        }
    }
}

impl Drop for OpenGLRenderer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteVertexArrays(1, &self.vao);
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteBuffers(1, &self.ebo);
            gl::DeleteTextures(1, &self.texture);
            gl::DeleteProgram(self.shader);
        }
    }
}

pub fn setup_opengl_context(
    window: Arc<Window>,
) -> (PossiblyCurrentContext, Surface<WindowSurface>) {
    let window_handle = window.window_handle().unwrap();
    let display_handle = window.display_handle().unwrap();

    #[cfg(target_os = "macos")]
    let api_preference = DisplayApiPreference::Cgl;
    #[cfg(target_os = "windows")]
    let api_preference = DisplayApiPreference::Wgl(Some(window_handle.as_raw()));
    #[cfg(target_os = "linux")]
    let api_preference = DisplayApiPreference::EglThenGlx(Some(window_handle.as_raw()));

    let gl_display = unsafe { Display::new(display_handle.as_raw(), api_preference).unwrap() };

    let config_template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(false)
        .build();

    let config = unsafe {
        gl_display
            .find_configs(config_template)
            .unwrap()
            .next()
            .unwrap()
    };

    let context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version { major: 3, minor: 3 })))
        .build(Some(window_handle.as_raw()));

    let gl_context = unsafe {
        gl_display
            .create_context(&config, &context_attributes)
            .unwrap()
    };

    let window_size = window.inner_size();
    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        window_handle.as_raw(),
        NonZeroU32::new(window_size.width).unwrap(),
        NonZeroU32::new(window_size.height).unwrap(),
    );

    let gl_surface = unsafe {
        gl_display
            .create_window_surface(&config, &surface_attributes)
            .unwrap()
    };

    let gl_context = gl_context.make_current(&gl_surface).unwrap();

    gl::load_with(|symbol| {
        let symbol = CString::new(symbol).unwrap();
        gl_display.get_proc_address(&symbol).cast()
    });

    (gl_context, gl_surface)
}
