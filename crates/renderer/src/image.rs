use std::slice;

use wallpaper_media::{DecodedImage, DecodedVideoFrame, VideoPixelFormat};
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, Fxc::D3DCompile},
            Direct3D11::{
                D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
                D3D11_CULL_NONE, D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                D3D11_RASTERIZER_DESC, D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA,
                D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_VIEWPORT, ID3D11Buffer, ID3D11DepthStencilView, ID3D11DeviceContext,
                ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView,
                ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
            },
            Dxgi::{
                Common::{
                    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN,
                    DXGI_SAMPLE_DESC,
                },
                DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
                DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
            },
        },
        UI::WindowsAndMessaging::{GetClientRect, IsWindow},
    },
    core::{Interface, PCSTR, s},
};

use crate::{
    ColorAdjustments, D3d11Device, ImageTransform, NormalizedCrop, PlacementDimensions,
    PlacementMode, RendererError, calculate_transformed_image_placement,
};

const VERTEX_SHADER: &str = r#"
struct VertexOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

cbuffer PlacementData : register(b0) {
    float4 destination_top_left;
    float4 destination_top_right;
    float4 destination_bottom_right;
    float4 destination_bottom_left;
    float4 source_uv_rect;
    float4 color_controls;
    float4 color_mix;
    float4 color_balance;
};

VertexOutput vs_main(uint vertex_id : SV_VertexID) {
    const float2 coordinates[6] = {
        float2(0.0, 0.0),
        float2(1.0, 0.0),
        float2(1.0, 1.0),
        float2(0.0, 0.0),
        float2(1.0, 1.0),
        float2(0.0, 1.0)
    };
    VertexOutput output;
    float2 top = lerp(destination_top_left.xy, destination_top_right.xy, coordinates[vertex_id].x);
    float2 bottom = lerp(destination_bottom_left.xy, destination_bottom_right.xy, coordinates[vertex_id].x);
    output.position = float4(lerp(top, bottom, coordinates[vertex_id].y), 0.0, 1.0);
    output.uv = float2(
        lerp(source_uv_rect.x, source_uv_rect.z, coordinates[vertex_id].x),
        lerp(source_uv_rect.y, source_uv_rect.w, coordinates[vertex_id].y)
    );
    return output;
}
"#;

const PIXEL_SHADER: &str = r#"
Texture2D source_image : register(t0);
SamplerState image_sampler : register(s0);

cbuffer ColorData : register(b0) {
    float4 destination_top_left;
    float4 destination_top_right;
    float4 destination_bottom_right;
    float4 destination_bottom_left;
    float4 source_uv_rect;
    float4 color_controls;
    float4 color_mix;
    float4 color_balance;
};

float3 srgb_to_linear(float3 color) {
    float3 low = color / 12.92;
    float3 high = pow(max((color + 0.055) / 1.055, 0.0), 2.4);
    return lerp(low, high, step(0.04045, color));
}

float3 linear_to_srgb(float3 color) {
    color = saturate(color);
    float3 low = color * 12.92;
    float3 high = 1.055 * pow(color, 1.0 / 2.4) - 0.055;
    return saturate(lerp(low, high, step(0.0031308, color)));
}

float4 ps_main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float4 sampled_color = source_image.Sample(image_sampler, uv);
    if (all(color_controls == float4(1.0, 1.0, 1.0, 1.0))
        && all(color_mix == float4(1.0, 0.0, 1.0, 1.0))
        && color_balance.x == 1.0) {
        return sampled_color;
    }

    float3 rgb = srgb_to_linear(sampled_color.rgb);
    float warm = color_mix.y * 0.1;
    rgb *= float3(
        color_mix.z * (1.0 + warm),
        color_mix.w,
        color_balance.x * (1.0 - warm)
    );
    rgb *= color_controls.x;
    rgb = (rgb - 0.18) * color_controls.y + 0.18;
    rgb = saturate(rgb);
    float luminance = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    rgb = saturate(lerp(luminance.xxx, rgb, color_controls.z));
    rgb = pow(rgb, 1.0 / color_controls.w);
    rgb *= color_mix.x;
    return float4(linear_to_srgb(rgb), 1.0);
}
"#;

/// Presents one decoded still image to a non-owning HWND.
///
/// The caller retains HWND ownership and must drop this presenter before destroying the window.
/// No render loop is started; frames are presented only on construction, image replacement, or
/// an observed window resize.
pub struct D3d11ImagePresenter<'a> {
    device: &'a D3d11Device,
    hwnd: HWND,
    swap_chain: IDXGISwapChain1,
    render_target: Option<ID3D11RenderTargetView>,
    placement_buffer: ID3D11Buffer,
    image_texture: ID3D11Texture2D,
    image_view: ID3D11ShaderResourceView,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    rasterizer: ID3D11RasterizerState,
    width: u32,
    height: u32,
    source_dimensions: PlacementDimensions,
    placement_mode: PlacementMode,
    transform: ImageTransform,
    color_adjustments: ColorAdjustments,
    presented_frames: u64,
}

struct Rgba8PixelBuffer<'pixels> {
    width: u32,
    height: u32,
    stride: u32,
    pixels: &'pixels [u8],
}

impl<'a> D3d11ImagePresenter<'a> {
    pub fn new(
        device: &'a D3d11Device,
        window_handle: isize,
        image: &DecodedImage,
    ) -> Result<Self, RendererError> {
        Self::new_with_placement(device, window_handle, image, PlacementMode::Cover)
    }

    /// Create the presenter with a specific scale and crop mode.
    pub fn new_with_placement(
        device: &'a D3d11Device,
        window_handle: isize,
        image: &DecodedImage,
        placement_mode: PlacementMode,
    ) -> Result<Self, RendererError> {
        Self::new_with_transform(
            device,
            window_handle,
            image,
            placement_mode,
            ImageTransform::default(),
        )
    }

    /// Create the presenter with placement and runtime transform parameters.
    pub fn new_with_transform(
        device: &'a D3d11Device,
        window_handle: isize,
        image: &DecodedImage,
        placement_mode: PlacementMode,
        transform: ImageTransform,
    ) -> Result<Self, RendererError> {
        Self::new_with_settings(
            device,
            window_handle,
            image,
            placement_mode,
            transform,
            ColorAdjustments::default(),
        )
    }

    /// Create a presenter with runtime color adjustments and neutral transform settings.
    pub fn new_with_color_adjustments(
        device: &'a D3d11Device,
        window_handle: isize,
        image: &DecodedImage,
        color_adjustments: ColorAdjustments,
    ) -> Result<Self, RendererError> {
        Self::new_with_settings(
            device,
            window_handle,
            image,
            PlacementMode::Cover,
            ImageTransform::default(),
            color_adjustments,
        )
    }

    fn new_with_settings(
        device: &'a D3d11Device,
        window_handle: isize,
        image: &DecodedImage,
        placement_mode: PlacementMode,
        transform: ImageTransform,
        color_adjustments: ColorAdjustments,
    ) -> Result<Self, RendererError> {
        Self::new_with_pixels(
            device,
            window_handle,
            Rgba8PixelBuffer {
                width: image.width(),
                height: image.height(),
                stride: image.stride(),
                pixels: image.rgba8(),
            },
            placement_mode,
            transform,
            color_adjustments,
        )
    }

    /// Create a presenter from the first decoded video frame.
    pub fn new_with_video_frame(
        device: &'a D3d11Device,
        window_handle: isize,
        frame: &DecodedVideoFrame,
    ) -> Result<Self, RendererError> {
        if frame.pixel_format() != VideoPixelFormat::Rgba8 {
            return Err(RendererError::InvalidVideoFrame);
        }
        Self::new_with_pixels(
            device,
            window_handle,
            Rgba8PixelBuffer {
                width: frame.width(),
                height: frame.height(),
                stride: frame.stride(),
                pixels: frame.rgba8(),
            },
            PlacementMode::Cover,
            ImageTransform::default(),
            ColorAdjustments::default(),
        )
    }

    fn new_with_pixels(
        device: &'a D3d11Device,
        window_handle: isize,
        pixels: Rgba8PixelBuffer<'_>,
        placement_mode: PlacementMode,
        transform: ImageTransform,
        color_adjustments: ColorAdjustments,
    ) -> Result<Self, RendererError> {
        validate_pixel_layout(pixels.width, pixels.height, pixels.stride, pixels.pixels)
            .map_err(|_| RendererError::InvalidVideoFrame)?;
        let hwnd = hwnd_from_raw(window_handle);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(RendererError::WindowUnavailable);
        }
        let (width, height) = client_size(hwnd)?;
        let source_dimensions = PlacementDimensions::new(pixels.width, pixels.height)?;
        let output_dimensions = PlacementDimensions::new(width, height)?;
        let dxgi_device: IDXGIDevice =
            device
                .device()
                .cast()
                .map_err(|error| RendererError::SwapChainCreation {
                    hresult: error.code().0,
                })?;
        let adapter = unsafe { dxgi_device.GetAdapter() }.map_err(|error| {
            RendererError::SwapChainCreation {
                hresult: error.code().0,
            }
        })?;
        let factory: IDXGIFactory2 =
            unsafe { adapter.GetParent() }.map_err(|error| RendererError::SwapChainCreation {
                hresult: error.code().0,
            })?;
        let swap_chain_description = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        let swap_chain = unsafe {
            factory.CreateSwapChainForHwnd(
                device.device(),
                hwnd,
                &swap_chain_description,
                None,
                None,
            )
        }
        .map_err(|error| RendererError::SwapChainCreation {
            hresult: error.code().0,
        })?;
        unsafe { factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER) }.map_err(|error| {
            RendererError::SwapChainCreation {
                hresult: error.code().0,
            }
        })?;

        let (image_texture, image_view) = create_rgba_texture(
            device,
            pixels.width,
            pixels.height,
            pixels.stride,
            pixels.pixels,
        )?;
        let placement_buffer = create_placement_buffer(
            device,
            source_dimensions,
            output_dimensions,
            placement_mode,
            transform,
            color_adjustments,
        )?;
        let vertex_shader = create_vertex_shader(device, VERTEX_SHADER)?;
        let pixel_shader = create_pixel_shader(device, PIXEL_SHADER)?;
        let sampler = create_sampler(device)?;
        let rasterizer = create_rasterizer(device)?;

        let mut presenter = Self {
            device,
            hwnd,
            swap_chain,
            render_target: None,
            placement_buffer,
            image_texture,
            image_view,
            vertex_shader,
            pixel_shader,
            sampler,
            rasterizer,
            width,
            height,
            source_dimensions,
            placement_mode,
            transform,
            color_adjustments,
            presented_frames: 0,
        };
        presenter.create_render_target()?;
        presenter.render_current_image()?;
        Ok(presenter)
    }

    /// Replace the current frame pixels and present once, reusing an equal-sized texture.
    pub fn set_image(&mut self, image: &DecodedImage) -> Result<(), RendererError> {
        self.set_rgba_frame(image.width(), image.height(), image.stride(), image.rgba8())
    }

    /// Upload and present a decoded video frame through this presenter's existing swapchain.
    pub fn set_video_frame(&mut self, frame: &DecodedVideoFrame) -> Result<(), RendererError> {
        if frame.pixel_format() != VideoPixelFormat::Rgba8 {
            return Err(RendererError::InvalidVideoFrame);
        }
        self.set_rgba_frame(frame.width(), frame.height(), frame.stride(), frame.rgba8())
    }

    fn set_rgba_frame(
        &mut self,
        frame_width: u32,
        frame_height: u32,
        frame_stride: u32,
        frame_pixels: &[u8],
    ) -> Result<(), RendererError> {
        validate_pixel_layout(frame_width, frame_height, frame_stride, frame_pixels)
            .map_err(|_| RendererError::InvalidVideoFrame)?;
        let new_dimensions = PlacementDimensions::new(frame_width, frame_height)
            .map_err(|_| RendererError::InvalidVideoFrame)?;

        if new_dimensions == self.source_dimensions {
            self.unbind_resources();
            let depth_pitch = frame_stride
                .checked_mul(frame_height)
                .ok_or(RendererError::InvalidVideoFrame)?;
            unsafe {
                self.device.immediate_context().UpdateSubresource(
                    &self.image_texture,
                    0,
                    None,
                    frame_pixels.as_ptr().cast(),
                    frame_stride,
                    depth_pitch,
                );
            }
        } else {
            let (texture, view) = create_rgba_texture(
                self.device,
                frame_width,
                frame_height,
                frame_stride,
                frame_pixels,
            )?;
            let placement_buffer = create_placement_buffer(
                self.device,
                new_dimensions,
                PlacementDimensions::new(self.width, self.height)?,
                self.placement_mode,
                self.transform,
                self.color_adjustments,
            )?;
            self.unbind_resources();
            self.image_texture = texture;
            self.image_view = view;
            self.source_dimensions = new_dimensions;
            self.placement_buffer = placement_buffer;
        }
        self.render_current_image()
    }

    /// Change placement mode, drawing one frame only when the mode changes.
    pub fn set_placement_mode(
        &mut self,
        placement_mode: PlacementMode,
    ) -> Result<(), RendererError> {
        if placement_mode == self.placement_mode {
            return Ok(());
        }
        let placement_buffer = create_placement_buffer(
            self.device,
            self.source_dimensions,
            PlacementDimensions::new(self.width, self.height)?,
            placement_mode,
            self.transform,
            self.color_adjustments,
        )?;
        self.render_with_placement_buffer(&placement_buffer)?;
        self.placement_buffer = placement_buffer;
        self.placement_mode = placement_mode;
        Ok(())
    }

    /// Apply runtime transform parameters and present one frame only if they changed.
    pub fn set_transform(&mut self, transform: ImageTransform) -> Result<(), RendererError> {
        if transform == self.transform {
            return Ok(());
        }
        let placement_buffer = create_placement_buffer(
            self.device,
            self.source_dimensions,
            PlacementDimensions::new(self.width, self.height)?,
            self.placement_mode,
            transform,
            self.color_adjustments,
        )?;
        self.render_with_placement_buffer(&placement_buffer)?;
        self.placement_buffer = placement_buffer;
        self.transform = transform;
        Ok(())
    }

    pub const fn transform(&self) -> ImageTransform {
        self.transform
    }

    pub const fn placement_mode(&self) -> PlacementMode {
        self.placement_mode
    }

    /// Apply runtime color parameters and present one frame only when they changed.
    pub fn set_color_adjustments(
        &mut self,
        color_adjustments: ColorAdjustments,
    ) -> Result<(), RendererError> {
        if color_adjustments == self.color_adjustments {
            return Ok(());
        }
        let placement_buffer = create_placement_buffer(
            self.device,
            self.source_dimensions,
            PlacementDimensions::new(self.width, self.height)?,
            self.placement_mode,
            self.transform,
            color_adjustments,
        )?;
        self.render_with_placement_buffer(&placement_buffer)?;
        self.placement_buffer = placement_buffer;
        self.color_adjustments = color_adjustments;
        Ok(())
    }

    /// Apply shared editor color settings through the D3D11 color shader.
    pub fn set_editor_color_settings(
        &mut self,
        settings: wallpaper_core::editor_settings::EditorColorSettings,
    ) -> Result<(), RendererError> {
        self.set_color_adjustments(ColorAdjustments::from_editor_settings(settings)?)
    }

    /// Apply one complete editor snapshot, presenting it before committing presenter state.
    pub fn set_editor_settings(
        &mut self,
        settings: &wallpaper_core::editor_settings::WallpaperEditorSettings,
    ) -> Result<(), RendererError> {
        use wallpaper_core::editor_settings::PlacementMode as StoredPlacementMode;

        settings
            .validate()
            .map_err(|_| RendererError::InvalidEditorSettings)?;
        let render = settings.render();
        let placement_mode = match render.placement_mode() {
            StoredPlacementMode::Cover | StoredPlacementMode::Custom => PlacementMode::Cover,
            StoredPlacementMode::Fit => PlacementMode::Fit,
            StoredPlacementMode::Stretch => PlacementMode::Stretch,
            StoredPlacementMode::Center => PlacementMode::Center,
        };
        let position = render.position_px();
        let scale = render.scale();
        let anchor = render.anchor();
        let crop = render.crop();
        let size_px = render
            .size_px()
            .map(|size| (size.width() as f32, size.height() as f32));
        let transform = ImageTransform::new(
            (position.x() as f32, position.y() as f32),
            (scale.x(), scale.y()),
            render.rotation_degrees(),
            (anchor.x(), anchor.y()),
            NormalizedCrop::new(crop.x(), crop.y(), crop.width(), crop.height())?,
        )?
        .with_size_px(size_px)?;
        let color_adjustments = ColorAdjustments::from_editor_settings(settings.color())?;
        let placement_buffer = create_placement_buffer(
            self.device,
            self.source_dimensions,
            PlacementDimensions::new(self.width, self.height)?,
            placement_mode,
            transform,
            color_adjustments,
        )?;

        self.render_with_placement_buffer(&placement_buffer)?;
        self.placement_buffer = placement_buffer;
        self.placement_mode = placement_mode;
        self.transform = transform;
        self.color_adjustments = color_adjustments;
        Ok(())
    }

    pub const fn color_adjustments(&self) -> ColorAdjustments {
        self.color_adjustments
    }

    /// Resize swap-chain buffers and present only when the child client size changed.
    pub fn refresh_if_resized(&mut self) -> Result<bool, RendererError> {
        if !unsafe { IsWindow(Some(self.hwnd)) }.as_bool() {
            return Err(RendererError::WindowUnavailable);
        }
        let (width, height) = client_size(self.hwnd)?;
        if width == self.width && height == self.height {
            return Ok(false);
        }
        let placement_buffer = create_placement_buffer(
            self.device,
            self.source_dimensions,
            PlacementDimensions::new(width, height)?,
            self.placement_mode,
            self.transform,
            self.color_adjustments,
        )?;

        unsafe {
            self.device
                .immediate_context()
                .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
        self.render_target = None;
        unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        }
        .map_err(|error| RendererError::SwapChainResize {
            hresult: error.code().0,
        })?;
        self.width = width;
        self.height = height;
        self.placement_buffer = placement_buffer;
        self.create_render_target()?;
        self.render_current_image()?;
        Ok(true)
    }

    /// Number of successful explicit or resize-triggered presentations.
    pub const fn presented_frames(&self) -> u64 {
        self.presented_frames
    }

    fn create_render_target(&mut self) -> Result<(), RendererError> {
        let back_buffer =
            unsafe { self.swap_chain.GetBuffer::<ID3D11Texture2D>(0) }.map_err(|error| {
                RendererError::BackBufferAccess {
                    hresult: error.code().0,
                }
            })?;
        let mut render_target = None;
        unsafe {
            self.device.device().CreateRenderTargetView(
                &back_buffer,
                None,
                Some(&mut render_target),
            )
        }
        .map_err(|error| RendererError::RenderTargetCreation {
            hresult: error.code().0,
        })?;
        self.render_target =
            Some(render_target.ok_or(RendererError::RenderTargetCreation { hresult: 0 })?);
        Ok(())
    }

    fn render_current_image(&mut self) -> Result<(), RendererError> {
        let placement_buffer = self.placement_buffer.clone();
        self.render_with_placement_buffer(&placement_buffer)
    }

    fn render_with_placement_buffer(
        &mut self,
        placement_buffer: &ID3D11Buffer,
    ) -> Result<(), RendererError> {
        self.draw_with_placement_buffer(placement_buffer)?;
        unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|error| RendererError::Present {
                hresult: error.code().0,
            })?;
        self.presented_frames = self.presented_frames.saturating_add(1);
        Ok(())
    }

    fn draw_with_placement_buffer(
        &self,
        placement_buffer: &ID3D11Buffer,
    ) -> Result<(), RendererError> {
        if !unsafe { IsWindow(Some(self.hwnd)) }.as_bool() {
            return Err(RendererError::WindowUnavailable);
        }
        let render_target = self
            .render_target
            .as_ref()
            .ok_or(RendererError::BackBufferAccess { hresult: 0 })?;
        let context: &ID3D11DeviceContext = self.device.immediate_context();
        let targets = [Some(render_target.clone())];
        let image_views = [Some(self.image_view.clone())];
        let samplers = [Some(self.sampler.clone())];
        let placement_buffers = [Some(placement_buffer.clone())];
        let viewport = [D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: self.width as f32,
            Height: self.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }];

        unsafe {
            context.ClearRenderTargetView(render_target, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(&targets), None::<&ID3D11DepthStencilView>);
            context.RSSetViewports(Some(&viewport));
            context.RSSetState(&self.rasterizer);
            context
                .IASetInputLayout(None::<&windows::Win32::Graphics::Direct3D11::ID3D11InputLayout>);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vertex_shader, None);
            context.VSSetConstantBuffers(0, Some(&placement_buffers));
            context.PSSetShader(&self.pixel_shader, None);
            context.PSSetConstantBuffers(0, Some(&placement_buffers));
            context.PSSetShaderResources(0, Some(&image_views));
            context.PSSetSamplers(0, Some(&samplers));
            context.Draw(6, 0);
            context.PSSetShaderResources(0, Some(&[None]));
            context.VSSetConstantBuffers(0, Some(&[None]));
            context.PSSetConstantBuffers(0, Some(&[None]));
            context.OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
        Ok(())
    }

    #[cfg(test)]
    fn draw_current_frame_for_readback(&self) -> Result<(), RendererError> {
        self.draw_with_placement_buffer(&self.placement_buffer)
    }

    fn unbind_resources(&self) {
        let context = self.device.immediate_context();
        unsafe {
            context.PSSetShaderResources(0, Some(&[None]));
            context.VSSetConstantBuffers(0, Some(&[None]));
            context.PSSetConstantBuffers(0, Some(&[None]));
            context.PSSetSamplers(0, Some(&[None]));
            context.VSSetShader(None::<&ID3D11VertexShader>, None);
            context.PSSetShader(None::<&ID3D11PixelShader>, None);
            context.RSSetState(None::<&ID3D11RasterizerState>);
            context.OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
    }
}

impl Drop for D3d11ImagePresenter<'_> {
    fn drop(&mut self) {
        self.unbind_resources();
    }
}

fn validate_pixel_layout(
    width: u32,
    height: u32,
    stride: u32,
    pixels: &[u8],
) -> Result<(), RendererError> {
    if width == 0 || height == 0 {
        return Err(RendererError::InvalidVideoFrame);
    }
    let row_bytes = width
        .checked_mul(4)
        .ok_or(RendererError::InvalidVideoFrame)?;
    if stride < row_bytes {
        return Err(RendererError::InvalidVideoFrame);
    }
    let required = usize::try_from(
        stride
            .checked_mul(height)
            .ok_or(RendererError::InvalidVideoFrame)?,
    )
    .map_err(|_| RendererError::InvalidVideoFrame)?;
    if pixels.len() < required {
        return Err(RendererError::InvalidVideoFrame);
    }
    Ok(())
}

fn create_rgba_texture(
    device: &D3d11Device,
    width: u32,
    height: u32,
    stride: u32,
    pixels: &[u8],
) -> Result<(ID3D11Texture2D, ID3D11ShaderResourceView), RendererError> {
    validate_pixel_layout(width, height, stride, pixels)?;
    let byte_len = stride
        .checked_mul(height)
        .ok_or(RendererError::ImageTextureCreation { hresult: 0 })?;
    let subresource = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: stride,
        SysMemSlicePitch: byte_len,
    };
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe {
        device
            .device()
            .CreateTexture2D(&description, Some(&subresource), Some(&mut texture))
    }
    .map_err(|error| RendererError::ImageTextureCreation {
        hresult: error.code().0,
    })?;
    let texture = texture.ok_or(RendererError::ImageTextureCreation { hresult: 0 })?;
    let mut view = None;
    unsafe {
        device
            .device()
            .CreateShaderResourceView(&texture, None, Some(&mut view))
    }
    .map_err(|error| RendererError::ImageViewCreation {
        hresult: error.code().0,
    })?;
    let view = view.ok_or(RendererError::ImageViewCreation { hresult: 0 })?;
    Ok((texture, view))
}

#[repr(C)]
struct PlacementConstants {
    destination_top_left: [f32; 4],
    destination_top_right: [f32; 4],
    destination_bottom_right: [f32; 4],
    destination_bottom_left: [f32; 4],
    source_uv: [f32; 4],
    color_controls: [f32; 4],
    color_mix: [f32; 4],
    color_balance: [f32; 4],
}

fn create_placement_buffer(
    device: &D3d11Device,
    source: PlacementDimensions,
    output: PlacementDimensions,
    mode: PlacementMode,
    transform: ImageTransform,
    color_adjustments: ColorAdjustments,
) -> Result<ID3D11Buffer, RendererError> {
    let placement = calculate_transformed_image_placement(source, output, mode, transform)?;
    let corners = placement.destination_corners();
    let clip_corners = [
        clip_point(corners[0].x(), corners[0].y())?,
        clip_point(corners[1].x(), corners[1].y())?,
        clip_point(corners[2].x(), corners[2].y())?,
        clip_point(corners[3].x(), corners[3].y())?,
    ];
    let source_uv = placement.source_uv();
    let constants = PlacementConstants {
        destination_top_left: clip_corners[0],
        destination_top_right: clip_corners[1],
        destination_bottom_right: clip_corners[2],
        destination_bottom_left: clip_corners[3],
        source_uv: [
            source_uv.left(),
            source_uv.top(),
            source_uv.right(),
            source_uv.bottom(),
        ],
        color_controls: [
            color_adjustments.brightness(),
            color_adjustments.contrast(),
            color_adjustments.saturation(),
            color_adjustments.gamma(),
        ],
        color_mix: [
            color_adjustments.opacity(),
            color_adjustments.temperature(),
            color_adjustments.rgb_balance().0,
            color_adjustments.rgb_balance().1,
        ],
        color_balance: [color_adjustments.rgb_balance().2, 0.0, 0.0, 0.0],
    };
    let subresource = D3D11_SUBRESOURCE_DATA {
        pSysMem: (&constants as *const PlacementConstants).cast(),
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let description = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<PlacementConstants>() as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer = None;
    unsafe {
        device
            .device()
            .CreateBuffer(&description, Some(&subresource), Some(&mut buffer))
    }
    .map_err(|error| RendererError::PlacementBufferCreation {
        hresult: error.code().0,
    })?;
    buffer.ok_or(RendererError::PlacementBufferCreation { hresult: 0 })
}

fn clip_point(x: f32, y: f32) -> Result<[f32; 4], RendererError> {
    let x = f64::from(x) * 2.0 - 1.0;
    let y = 1.0 - f64::from(y) * 2.0;
    if !x.is_finite()
        || !y.is_finite()
        || x.abs() > f64::from(f32::MAX)
        || y.abs() > f64::from(f32::MAX)
    {
        return Err(RendererError::InvalidTransformParameters);
    }
    Ok([x as f32, y as f32, 0.0, 0.0])
}

fn create_vertex_shader(
    device: &D3d11Device,
    source: &str,
) -> Result<ID3D11VertexShader, RendererError> {
    let bytecode = compile_shader(source, s!("vs_main"), s!("vs_4_0"))?;
    let mut shader = None;
    unsafe {
        device
            .device()
            .CreateVertexShader(&bytecode, None, Some(&mut shader))
    }
    .map_err(|error| RendererError::ShaderCreation {
        hresult: error.code().0,
    })?;
    shader.ok_or(RendererError::ShaderCreation { hresult: 0 })
}

fn create_pixel_shader(
    device: &D3d11Device,
    source: &str,
) -> Result<ID3D11PixelShader, RendererError> {
    let bytecode = compile_shader(source, s!("ps_main"), s!("ps_4_0"))?;
    let mut shader = None;
    unsafe {
        device
            .device()
            .CreatePixelShader(&bytecode, None, Some(&mut shader))
    }
    .map_err(|error| RendererError::ShaderCreation {
        hresult: error.code().0,
    })?;
    shader.ok_or(RendererError::ShaderCreation { hresult: 0 })
}

fn compile_shader(source: &str, entry: PCSTR, profile: PCSTR) -> Result<Vec<u8>, RendererError> {
    use windows::Win32::Graphics::Direct3D::Fxc::D3DCOMPILE_ENABLE_STRICTNESS;

    let mut code = None;
    unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None,
            entry,
            profile,
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut code,
            None,
        )
    }
    .map_err(|error| RendererError::ShaderCompilation {
        hresult: error.code().0,
    })?;
    let code = code.ok_or(RendererError::ShaderCompilation { hresult: 0 })?;
    let bytes = unsafe {
        slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize())
    };
    Ok(bytes.to_vec())
}

fn create_sampler(device: &D3d11Device) -> Result<ID3D11SamplerState, RendererError> {
    let description = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
        ..Default::default()
    };
    let mut sampler = None;
    unsafe {
        device
            .device()
            .CreateSamplerState(&description, Some(&mut sampler))
    }
    .map_err(|error| RendererError::SamplerCreation {
        hresult: error.code().0,
    })?;
    sampler.ok_or(RendererError::SamplerCreation { hresult: 0 })
}

fn create_rasterizer(device: &D3d11Device) -> Result<ID3D11RasterizerState, RendererError> {
    let description = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        DepthClipEnable: true.into(),
        ..Default::default()
    };
    let mut rasterizer = None;
    unsafe {
        device
            .device()
            .CreateRasterizerState(&description, Some(&mut rasterizer))
    }
    .map_err(|error| RendererError::RasterizerCreation {
        hresult: error.code().0,
    })?;
    rasterizer.ok_or(RendererError::RasterizerCreation { hresult: 0 })
}

fn client_size(hwnd: HWND) -> Result<(u32, u32), RendererError> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.map_err(|_| RendererError::WindowSizeUnavailable)?;
    let width = u32::try_from(rect.right - rect.left)
        .ok()
        .filter(|width| *width > 0)
        .ok_or(RendererError::WindowSizeUnavailable)?;
    let height = u32::try_from(rect.bottom - rect.top)
        .ok()
        .filter(|height| *height > 0)
        .ok_or(RendererError::WindowSizeUnavailable)?;
    Ok((width, height))
}

fn hwnd_from_raw(raw_handle: isize) -> HWND {
    HWND(raw_handle as *mut _)
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use wallpaper_desktop::discover_desktop_host;
    use wallpaper_media::{
        DecodedImage, DecodedVideoFrame, VideoReadEvent, VideoSource, decode_image,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_STAGING, ID3D11Buffer, ID3D11DepthStencilView, ID3D11PixelShader,
        ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
        ID3D11VertexShader,
    };
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetWindowPos};
    use windows::core::Interface;

    use super::{D3d11ImagePresenter, hwnd_from_raw};
    use crate::D3d11Device;
    use crate::color::apply_rgba8_reference;
    use crate::{
        ColorAdjustments, ImageTransform, NormalizedCrop, PlacementDimensions, PlacementMode,
        calculate_transformed_image_placement,
    };
    use windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE;

    fn read_render_target_pixel(presenter: &D3d11ImagePresenter<'_>, x: u32, y: u32) -> [u8; 4] {
        presenter
            .draw_current_frame_for_readback()
            .expect("current image should be drawn for readback");
        let render_target = presenter
            .render_target
            .as_ref()
            .expect("presenter should have a render target");
        let resource = unsafe { render_target.GetResource() }
            .expect("render target resource should be available");
        let texture: ID3D11Texture2D = resource
            .cast()
            .expect("render target should be a 2D texture");
        let mut description = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut description) };
        assert!(x < description.Width && y < description.Height);

        description.Usage = D3D11_USAGE_STAGING;
        description.BindFlags = 0;
        description.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        description.MiscFlags = 0;
        let mut staging = None;
        unsafe {
            presenter
                .device
                .device()
                .CreateTexture2D(&description, None, Some(&mut staging))
        }
        .expect("staging texture should be created");
        let staging = staging.expect("staging texture should be returned");
        let context = presenter.device.immediate_context();
        unsafe {
            context.CopyResource(&staging, &texture);
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
            .expect("staging texture should be readable");
        let offset = y as usize * mapped.RowPitch as usize + x as usize * 4;
        let pixel =
            unsafe { std::slice::from_raw_parts((mapped.pData as *const u8).add(offset), 4) };
        let result = [pixel[0], pixel[1], pixel[2], pixel[3]];
        unsafe { context.Unmap(&staging, 0) };
        result
    }

    fn read_render_target_rgba8(presenter: &D3d11ImagePresenter<'_>) -> Vec<u8> {
        presenter
            .draw_current_frame_for_readback()
            .expect("current image should be drawn for readback");
        let render_target = presenter
            .render_target
            .as_ref()
            .expect("presenter should have a render target");
        let resource = unsafe { render_target.GetResource() }
            .expect("render target resource should be available");
        let texture: ID3D11Texture2D = resource
            .cast()
            .expect("render target should be a 2D texture");
        let mut description = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut description) };
        description.Usage = D3D11_USAGE_STAGING;
        description.BindFlags = 0;
        description.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        description.MiscFlags = 0;
        let mut staging = None;
        unsafe {
            presenter
                .device
                .device()
                .CreateTexture2D(&description, None, Some(&mut staging))
        }
        .expect("staging texture should be created");
        let staging = staging.expect("staging texture should be returned");
        let context = presenter.device.immediate_context();
        unsafe { context.CopyResource(&staging, &texture) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
            .expect("staging texture should be readable");
        let mut result = vec![0; description.Width as usize * description.Height as usize * 4];
        for row in 0..description.Height as usize {
            let source = unsafe {
                std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(row * mapped.RowPitch as usize),
                    description.Width as usize * 4,
                )
            };
            let start = row * description.Width as usize * 4;
            result[start..start + source.len()].copy_from_slice(source);
        }
        unsafe { context.Unmap(&staging, 0) };
        result
    }

    fn assert_presenter_resources_are_unbound(device: &D3d11Device) {
        let context = device.immediate_context();
        let mut render_targets = [None::<ID3D11RenderTargetView>];
        let mut depth_stencil = None;
        let mut image_views = [None::<ID3D11ShaderResourceView>];
        let mut vertex_buffers = [None::<ID3D11Buffer>];
        let mut pixel_buffers = [None::<ID3D11Buffer>];
        let mut vertex_shader = None::<ID3D11VertexShader>;
        let mut pixel_shader = None::<ID3D11PixelShader>;
        let mut samplers = [None::<ID3D11SamplerState>];
        unsafe {
            context.OMGetRenderTargets(Some(&mut render_targets), Some(&mut depth_stencil));
            context.PSGetShaderResources(0, Some(&mut image_views));
            context.VSGetConstantBuffers(0, Some(&mut vertex_buffers));
            context.PSGetConstantBuffers(0, Some(&mut pixel_buffers));
            context.VSGetShader(&mut vertex_shader, None, None);
            context.PSGetShader(&mut pixel_shader, None, None);
            context.PSGetSamplers(0, Some(&mut samplers));
        }
        let rasterizer = unsafe { context.RSGetState() };
        assert!(render_targets[0].is_none());
        assert!(depth_stencil.is_none());
        assert!(image_views[0].is_none());
        assert!(vertex_buffers[0].is_none());
        assert!(pixel_buffers[0].is_none());
        assert!(vertex_shader.is_none());
        assert!(pixel_shader.is_none());
        assert!(samplers[0].is_none());
        assert!(rasterizer.is_err());
    }

    fn expected_center_sample(image: &DecodedImage) -> [u8; 4] {
        let width = image.width() as usize;
        let height = image.height() as usize;
        let center_x = width / 2;
        let center_y = height / 2;
        let pixels = image.rgba8();
        let mut result = [0; 4];
        for channel in 0..4 {
            let sum: u32 = [
                (center_y - 1) * width + center_x - 1,
                (center_y - 1) * width + center_x,
                center_y * width + center_x - 1,
                center_y * width + center_x,
            ]
            .iter()
            .map(|pixel| u32::from(pixels[pixel * 4 + channel]))
            .sum();
            result[channel] = ((sum + 2) / 4) as u8;
        }
        result
    }

    #[test]
    fn clip_conversion_rejects_nonrepresentable_finite_coordinates() {
        assert!(matches!(
            super::clip_point(f32::MAX, 0.5),
            Err(crate::RendererError::InvalidTransformParameters)
        ));
        assert!(super::clip_point(0.0, 0.0).is_ok());
    }

    #[test]
    fn failed_geometry_updates_leave_state_retryable() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("pixels.png")).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("image should present");
        let original_hwnd = presenter.hwnd;
        let transform = ImageTransform::new(
            (8.0, -4.0),
            (1.25, 0.75),
            12.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("transform should be valid");

        presenter.hwnd = hwnd_from_raw(0);
        assert!(matches!(
            presenter.set_transform(transform),
            Err(crate::RendererError::WindowUnavailable)
        ));
        assert_eq!(presenter.transform(), ImageTransform::default());
        assert!(matches!(
            presenter.set_placement_mode(PlacementMode::Fit),
            Err(crate::RendererError::WindowUnavailable)
        ));
        assert_eq!(presenter.placement_mode(), PlacementMode::Cover);

        presenter.hwnd = original_hwnd;
        presenter
            .set_transform(transform)
            .expect("same transform should retry after the window is restored");
        presenter
            .set_placement_mode(PlacementMode::Fit)
            .expect("same mode should retry after the window is restored");
        assert_eq!(presenter.presented_frames(), 3);
    }

    #[test]
    fn color_adjustments_redraw_only_after_success_and_leave_source_unchanged() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let path = fixture_path("pixels.png");
        let source_before = std::fs::read(&path).expect("source fixture should be readable");
        let image = decode_image(&path).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("image should present");
        let color = ColorAdjustments::new(1.25, 1.1, 0.8, 1.2, 0.9, 0.25, (1.0, 0.95, 1.05))
            .expect("color adjustments should be valid");
        let original_hwnd = presenter.hwnd;

        presenter.hwnd = hwnd_from_raw(0);
        assert!(matches!(
            presenter.set_color_adjustments(color),
            Err(crate::RendererError::WindowUnavailable)
        ));
        assert_eq!(presenter.color_adjustments(), ColorAdjustments::default());
        presenter.hwnd = original_hwnd;
        presenter
            .set_color_adjustments(color)
            .expect("color settings should retry after the window is restored");
        assert_eq!(presenter.presented_frames(), 2);
        assert_eq!(presenter.color_adjustments(), color);
        presenter
            .set_color_adjustments(color)
            .expect("unchanged color settings should be a no-op");
        assert_eq!(presenter.presented_frames(), 2);
        assert_eq!(
            std::fs::read(&path).expect("source fixture should remain readable"),
            source_before
        );
    }

    #[test]
    fn gpu_color_output_matches_the_linear_srgb_reference() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("solid.jpg")).expect("JPEG fixture should decode");
        let source_pixel = expected_center_sample(&image);
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("JPEG fixture should present");
        let cases = [
            ColorAdjustments::default(),
            ColorAdjustments::new(2.0, 1.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0))
                .expect("brightness case should be valid"),
            ColorAdjustments::new(1.0, 1.6, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0))
                .expect("contrast case should be valid"),
            ColorAdjustments::new(1.0, 1.0, 0.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0))
                .expect("saturation case should be valid"),
            ColorAdjustments::new(1.0, 1.0, 1.0, 2.0, 1.0, 0.0, (1.0, 1.0, 1.0))
                .expect("gamma case should be valid"),
            ColorAdjustments::new(1.0, 1.0, 1.0, 1.0, 0.5, 0.0, (1.0, 1.0, 1.0))
                .expect("opacity case should be valid"),
            ColorAdjustments::new(1.0, 1.0, 1.0, 1.0, 1.0, 0.75, (1.0, 1.0, 1.0))
                .expect("temperature case should be valid"),
            ColorAdjustments::new(1.0, 1.0, 1.0, 1.0, 1.0, 0.0, (2.0, 1.0, 1.0))
                .expect("RGB balance case should be valid"),
            ColorAdjustments::new(1.25, 1.1, 0.8, 1.2, 0.9, 0.25, (1.0, 0.95, 1.05))
                .expect("combined case should be valid"),
        ];

        for adjustments in cases {
            presenter
                .set_color_adjustments(adjustments)
                .expect("GPU color settings should render");
            let expected = apply_rgba8_reference(source_pixel, adjustments);
            let actual =
                read_render_target_pixel(&presenter, presenter.width / 2, presenter.height / 2);
            for channel in 0..4 {
                assert!(
                    actual[channel].abs_diff(expected[channel]) <= 3,
                    "GPU/reference mismatch for {adjustments:?}, channel {channel}: actual {actual:?}, expected {expected:?}"
                );
            }
        }
    }

    #[test]
    fn gpu_placement_and_crop_output_matches_documented_geometry() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("pixels.png")).expect("PNG fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("PNG fixture should present");
        let center_x = presenter.width / 2;
        let center_y = presenter.height / 2;

        presenter
            .set_placement_mode(PlacementMode::Fit)
            .expect("Fit should render");
        let fit_bar = read_render_target_pixel(&presenter, center_x, 0);
        assert!(
            fit_bar[..3].iter().all(|channel| *channel <= 1),
            "{fit_bar:?}"
        );

        presenter
            .set_placement_mode(PlacementMode::Stretch)
            .expect("Stretch should render");
        let stretched_top = read_render_target_pixel(&presenter, center_x, 0);
        assert!(stretched_top[..3].iter().any(|channel| *channel > 8));

        presenter
            .set_placement_mode(PlacementMode::Cover)
            .expect("Cover should render");
        let crop = NormalizedCrop::new(0.0, 0.0, 0.5, 1.0).expect("left-half crop is valid");
        let transform = ImageTransform::new((0.0, 0.0), (1.0, 1.0), 0.0, (0.5, 0.5), crop)
            .expect("identity transform with crop is valid");
        presenter
            .set_transform(transform)
            .expect("cropped image should render");
        let cropped_center = read_render_target_pixel(&presenter, center_x, center_y);
        assert!(cropped_center[0] >= 250, "{cropped_center:?}");
        assert!(
            cropped_center[1] <= 4 && cropped_center[2] <= 4,
            "{cropped_center:?}"
        );

        let translated = ImageTransform::new(
            (400.0, 0.0),
            (1.0, 1.0),
            0.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("physical-pixel translation is valid");
        presenter
            .set_transform(translated)
            .expect("translated image should render");
        let uncovered_left = read_render_target_pixel(&presenter, 0, center_y);
        assert!(uncovered_left[..3].iter().all(|channel| *channel <= 1));

        let scaled = ImageTransform::new(
            (0.0, 0.0),
            (0.5, 0.5),
            0.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("centered scale is valid");
        presenter
            .set_transform(scaled)
            .expect("scaled image should render");
        let scaled_uncovered = read_render_target_pixel(&presenter, 0, center_y);
        assert!(scaled_uncovered[..3].iter().all(|channel| *channel <= 1));

        let top_left_anchor = ImageTransform::new(
            (0.0, 0.0),
            (0.5, 0.5),
            0.0,
            (0.0, 0.0),
            NormalizedCrop::default(),
        )
        .expect("top-left anchor is valid");
        presenter
            .set_transform(top_left_anchor)
            .expect("top-left anchored image should render");
        let anchored_top_left = read_render_target_pixel(&presenter, 16, 16);
        assert!(anchored_top_left[..3].iter().any(|channel| *channel > 8));

        let rotated = ImageTransform::new(
            (0.0, 0.0),
            (1.0, 1.0),
            90.0,
            (0.5, 0.5),
            NormalizedCrop::default(),
        )
        .expect("quarter-turn rotation is valid");
        presenter
            .set_transform(rotated)
            .expect("rotated image should render");
        let rotated_top = read_render_target_pixel(&presenter, center_x, center_y - 100);
        let rotated_bottom = read_render_target_pixel(&presenter, center_x, center_y + 100);
        assert!(
            rotated_top[0] > rotated_bottom[0].saturating_add(20),
            "top={rotated_top:?}, bottom={rotated_bottom:?}"
        );

        let attached_hwnd = window.raw_handle();
        let context = device.immediate_context();
        let render_targets = [presenter.render_target.clone()];
        let image_views = [Some(presenter.image_view.clone())];
        let placement_buffers = [Some(presenter.placement_buffer.clone())];
        let samplers = [Some(presenter.sampler.clone())];
        unsafe {
            context.OMSetRenderTargets(Some(&render_targets), None::<&ID3D11DepthStencilView>);
            context.PSSetShaderResources(0, Some(&image_views));
            context.VSSetConstantBuffers(0, Some(&placement_buffers));
            context.PSSetConstantBuffers(0, Some(&placement_buffers));
            context.PSSetSamplers(0, Some(&samplers));
            context.VSSetShader(&presenter.vertex_shader, None);
            context.PSSetShader(&presenter.pixel_shader, None);
            context.RSSetState(&presenter.rasterizer);
        }
        drop(presenter);
        assert_presenter_resources_are_unbound(&device);
        drop(window);
        assert!(
            !unsafe { IsWindow(Some(hwnd_from_raw(attached_hwnd))) }.as_bool(),
            "the attached child window should be destroyed after presenter cleanup"
        );
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("media")
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn next_video_frame(source: &mut VideoSource, label: &str) -> DecodedVideoFrame {
        loop {
            let event = source
                .read_next()
                .unwrap_or_else(|error| panic!("{label} frame should decode: {error:?}"));
            match event {
                VideoReadEvent::Frame(frame) => return frame,
                VideoReadEvent::StreamTick { .. } => continue,
                VideoReadEvent::EndOfStream => panic!("fixture should contain another frame"),
            }
        }
    }

    #[test]
    fn video_frames_reuse_texture_and_swapchain_until_dimensions_change() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");

        let mut same_size_source = VideoSource::open(fixture_path("distinct-h264.mp4"))
            .expect("the 64x48 distinct-color H.264 fixture should open");
        let first = next_video_frame(&mut same_size_source, "first");
        let second = next_video_frame(&mut same_size_source, "second");
        assert_ne!(
            first.rgba8(),
            second.rgba8(),
            "fixture frames should differ"
        );
        let resized = next_video_frame(
            &mut VideoSource::open(fixture_path("short-h264-80x64.mp4"))
                .expect("the 80x64 H.264 fixture should open"),
            "resized",
        );
        assert_eq!((first.width(), first.height()), (64, 48));
        assert_eq!((resized.width(), resized.height()), (80, 64));

        let mut presenter =
            D3d11ImagePresenter::new_with_video_frame(&device, window.raw_handle(), &first)
                .expect("the first frame should initialize the existing child presenter");
        let swap_chain = presenter.swap_chain.as_raw();
        let same_size_texture = presenter.image_texture.as_raw();
        let center = (presenter.width / 2, presenter.height / 2);
        let placement_mode = PlacementMode::Fit;
        let transform = ImageTransform::new(
            (0.0, 0.0),
            (0.9, 0.9),
            5.0,
            (0.5, 0.5),
            crate::NormalizedCrop::default(),
        )
        .expect("video transform should be valid");
        let color_adjustments =
            ColorAdjustments::new(1.2, 1.0, 1.0, 1.0, 1.0, 0.0, (1.0, 1.0, 1.0))
                .expect("video color adjustments should be valid");
        presenter
            .set_placement_mode(placement_mode)
            .expect("placement mode should update");
        presenter
            .set_transform(transform)
            .expect("transform should update");
        presenter
            .set_color_adjustments(color_adjustments)
            .expect("color adjustments should update");
        let first_output = read_render_target_pixel(&presenter, center.0, center.1);
        let frames_before_update = presenter.presented_frames();

        presenter
            .set_video_frame(&second)
            .expect("the next frame should update the existing texture");
        let second_output = read_render_target_pixel(&presenter, center.0, center.1);
        assert_ne!(
            first_output, second_output,
            "the new frame should change GPU output"
        );
        assert_eq!(presenter.image_texture.as_raw(), same_size_texture);
        assert_eq!(presenter.swap_chain.as_raw(), swap_chain);
        assert_eq!(presenter.placement_mode, placement_mode);
        assert_eq!(presenter.transform, transform);
        assert_eq!(presenter.color_adjustments(), color_adjustments);
        assert_eq!(presenter.presented_frames(), frames_before_update + 1);

        presenter
            .set_video_frame(&resized)
            .expect("a dimension change should replace only source-sized resources");
        assert_ne!(presenter.image_texture.as_raw(), same_size_texture);
        assert_eq!(presenter.swap_chain.as_raw(), swap_chain);
        assert_eq!(presenter.placement_mode, placement_mode);
        assert_eq!(presenter.transform, transform);
        assert_eq!(presenter.color_adjustments(), color_adjustments);
        assert_eq!(presenter.source_dimensions.width(), 80);
        assert_eq!(presenter.source_dimensions.height(), 64);
        assert_eq!(presenter.presented_frames(), frames_before_update + 2);

        let attached_hwnd = window.raw_handle();
        drop(presenter);
        assert_presenter_resources_are_unbound(&device);
        drop(window);
        assert!(
            !unsafe { IsWindow(Some(hwnd_from_raw(attached_hwnd))) }.as_bool(),
            "the video presenter should release before its owned child window"
        );
    }

    #[test]
    fn editor_preview_matches_d3d_output_within_documented_tolerance() {
        use wallpaper_core::editor_settings::{
            EditorColorSettings, NormalizedCrop as StoredCrop, NormalizedPoint, PhysicalPosition,
            PhysicalSize, PlacementMode as StoredPlacementMode, RenderSettings, Scale2d,
            WallpaperEditorSettings,
        };

        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("pixels.png")).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("fixture should present");
        let hwnd = hwnd_from_raw(window.raw_handle());
        unsafe { SetWindowPos(hwnd, None, 0, 0, 1280, 720, SWP_NOACTIVATE) }
            .expect("the preview test window should resize");
        presenter
            .refresh_if_resized()
            .expect("the swap chain should resize");

        let stored_crop = StoredCrop::new(0.1, 0.1, 0.8, 0.8).unwrap();
        let position = PhysicalPosition::new(34, -16).unwrap();
        let size = Some(PhysicalSize::new(900, 500).unwrap());
        let scale = Scale2d::new(1.05, 0.95).unwrap();
        let anchor = NormalizedPoint::new(0.4, 0.6).unwrap();
        let render = RenderSettings::new(
            StoredPlacementMode::Cover,
            position,
            size,
            scale,
            4.0,
            anchor,
            stored_crop,
        )
        .unwrap();
        let color =
            EditorColorSettings::new(1.05, 1.1, 0.9, 1.1, 0.95, 0.05, 1.1, 0.95, 1.05).unwrap();
        let settings = WallpaperEditorSettings::new(1, render, color).unwrap();
        let d3d_crop = NormalizedCrop::new(0.1, 0.1, 0.8, 0.8).unwrap();
        let d3d_transform =
            ImageTransform::new((34.0, -16.0), (1.05, 0.95), 4.0, (0.4, 0.6), d3d_crop)
                .unwrap()
                .with_size_px(Some((900.0, 500.0)))
                .unwrap();
        presenter.set_transform(d3d_transform).unwrap();
        presenter
            .set_editor_color_settings(settings.color())
            .unwrap();

        let gpu = read_render_target_rgba8(&presenter);
        let cpu = crate::render_editor_preview(
            image.width(),
            image.height(),
            image.stride(),
            image.rgba8(),
            1280,
            720,
            &settings,
        )
        .expect("the editor preview should render");
        assert_eq!((cpu.width(), cpu.height()), (1280, 720));
        assert_eq!(gpu.len(), cpu.rgba8().len());
        let placement = calculate_transformed_image_placement(
            PlacementDimensions::new(image.width(), image.height()).unwrap(),
            PlacementDimensions::new(1280, 720).unwrap(),
            PlacementMode::Cover,
            d3d_transform,
        )
        .unwrap();
        let corners = placement.destination_corners();
        let p = (f64::from(corners[0].x()), f64::from(corners[0].y()));
        let ex = (
            f64::from(corners[1].x()) - p.0,
            f64::from(corners[1].y()) - p.1,
        );
        let ey = (
            f64::from(corners[3].x()) - p.0,
            f64::from(corners[3].y()) - p.1,
        );
        let determinant = ex.0 * ey.1 - ex.1 * ey.0;
        let ex_len = (ex.0 * 1280.0).hypot(ex.1 * 720.0);
        let ey_len = (ey.0 * 1280.0).hypot(ey.1 * 720.0);
        let (maximum_rgb_difference, pixel_index, channel) = gpu
            .chunks_exact(4)
            .zip(cpu.rgba8().chunks_exact(4))
            .enumerate()
            .flat_map(|(pixel, (gpu, cpu))| {
                (0..3).map(move |channel| (gpu[channel].abs_diff(cpu[channel]), pixel, channel))
            })
            .max_by_key(|item| item.0)
            .unwrap_or((0, 0, 0));
        let maximum_interior_difference = gpu
            .chunks_exact(4)
            .zip(cpu.rgba8().chunks_exact(4))
            .enumerate()
            .filter_map(|(pixel, (gpu, cpu))| {
                let x = (pixel % 1280) as f64 + 0.5;
                let y = (pixel / 1280) as f64 + 0.5;
                let dx = x / 1280.0 - p.0;
                let dy = y / 720.0 - p.1;
                let u = (dx * ey.1 - dy * ey.0) / determinant;
                let v = (ex.0 * dy - ex.1 * dx) / determinant;
                if u * ex_len < 2.0
                    || (1.0 - u) * ex_len < 2.0
                    || v * ey_len < 2.0
                    || (1.0 - v) * ey_len < 2.0
                {
                    return None;
                }
                Some((0..3).map(|c| gpu[c].abs_diff(cpu[c])).max().unwrap_or(0))
            })
            .max()
            .unwrap_or(0);
        assert!(
            maximum_interior_difference <= 5,
            "maximum interior RGB difference {maximum_interior_difference}; overall {maximum_rgb_difference} at ({}, {}) ch {channel}: GPU {:?}, CPU {:?}",
            pixel_index % 1280,
            pixel_index / 1280,
            &gpu[pixel_index * 4..pixel_index * 4 + 3],
            &cpu.rgba8()[pixel_index * 4..pixel_index * 4 + 3],
        );
        drop(presenter);
    }

    #[test]
    fn video_editor_preview_matches_d3d_output_within_documented_tolerance() {
        use wallpaper_core::editor_settings::{EditorColorSettings, WallpaperEditorSettings};

        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let video_path = fixture_path("short-h264-80x64.mp4");
        let mut preview_source = VideoSource::open(&video_path)
            .expect("the H.264 fixture should open for the editor preview");
        let preview_frame = next_video_frame(&mut preview_source, "editor preview");
        let preview_decoder_mode = preview_source.decoder_mode();
        let mut desktop_source = VideoSource::open_with_d3d11_device(&video_path, device.device())
            .expect("the H.264 fixture should open with the presenter device");
        let desktop_frame = next_video_frame(&mut desktop_source, "desktop presenter");
        let desktop_decoder_mode = desktop_source.decoder_mode();
        assert_eq!(
            (preview_frame.width(), preview_frame.height()),
            (desktop_frame.width(), desktop_frame.height())
        );
        let decoder_difference = preview_frame
            .rgba8()
            .iter()
            .zip(desktop_frame.rgba8())
            .map(|(preview, desktop)| preview.abs_diff(*desktop))
            .max()
            .unwrap_or(0);
        let mut presenter =
            D3d11ImagePresenter::new_with_video_frame(&device, window.raw_handle(), &desktop_frame)
                .expect("the decoded video frame should present");
        let hwnd = hwnd_from_raw(window.raw_handle());
        unsafe { SetWindowPos(hwnd, None, 0, 0, 1280, 720, SWP_NOACTIVATE) }
            .expect("the preview test window should resize");
        presenter
            .refresh_if_resized()
            .expect("the video swap chain should resize");

        let defaults = WallpaperEditorSettings::default_for(1)
            .expect("default editor settings should validate");
        let color = EditorColorSettings::new(1.1, 1.05, 0.95, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0)
            .expect("video editor color settings should validate");
        let settings = WallpaperEditorSettings::new(1, defaults.render(), color)
            .expect("video editor settings should validate");
        presenter
            .set_editor_settings(&settings)
            .expect("the desktop presenter should apply editor settings");
        let gpu = read_render_target_rgba8(&presenter);
        let preview_cpu = crate::render_editor_preview(
            preview_frame.width(),
            preview_frame.height(),
            preview_frame.stride(),
            preview_frame.rgba8(),
            1280,
            720,
            &settings,
        )
        .expect("the editor preview should render the decoded H.264 video frame");
        let desktop_cpu = crate::render_editor_preview(
            desktop_frame.width(),
            desktop_frame.height(),
            desktop_frame.stride(),
            desktop_frame.rgba8(),
            1280,
            720,
            &settings,
        )
        .expect("the CPU renderer should render the desktop-decoded video frame");
        assert_eq!((preview_cpu.width(), preview_cpu.height()), (1280, 720));
        assert_eq!((desktop_cpu.width(), desktop_cpu.height()), (1280, 720));
        assert_eq!(gpu.len(), preview_cpu.rgba8().len());

        let max_interior_difference = |cpu: &[u8]| {
            gpu.chunks_exact(4)
                .zip(cpu.chunks_exact(4))
                .enumerate()
                .filter_map(|(pixel, (gpu, cpu))| {
                    let x = pixel % 1280;
                    let y = pixel / 1280;
                    if x < 2 || x >= 1278 || y < 2 || y >= 718 {
                        return None;
                    }
                    Some(
                        (0..3)
                            .map(|channel| gpu[channel].abs_diff(cpu[channel]))
                            .max()
                            .unwrap_or(0),
                    )
                })
                .max()
                .unwrap_or(0)
        };
        let renderer_difference = max_interior_difference(desktop_cpu.rgba8());
        let editor_preview_difference = max_interior_difference(preview_cpu.rgba8());
        println!(
            "video decoder modes: preview={preview_decoder_mode:?}, desktop={desktop_decoder_mode:?}; source max channel delta={decoder_difference}/255; same-frame renderer delta={renderer_difference}/255; editor-to-desktop delta={editor_preview_difference}/255"
        );
        assert!(
            renderer_difference <= 5,
            "maximum same-frame video renderer difference {renderer_difference}/255"
        );
        assert!(
            editor_preview_difference <= 16,
            "maximum editor-to-desktop video difference {editor_preview_difference}/255"
        );
    }

    #[test]
    fn presents_once_and_stays_idle_until_image_or_size_changes() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("pixels.png")).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("image should present");
        assert_eq!(presenter.presented_frames(), 1);

        thread::sleep(Duration::from_millis(250));
        assert_eq!(presenter.presented_frames(), 1);
        assert!(
            !presenter
                .refresh_if_resized()
                .expect("unchanged window size should be readable")
        );
        assert_eq!(presenter.presented_frames(), 1);
    }

    #[test]
    fn resize_recreates_buffers_and_presents_at_the_new_client_size() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("pixels.png")).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("image should present");
        let hwnd = hwnd_from_raw(window.raw_handle());

        unsafe { SetWindowPos(hwnd, None, 0, 0, 800, 600, SWP_NOACTIVATE) }
            .expect("the test window should resize");

        assert!(
            presenter
                .refresh_if_resized()
                .expect("swap-chain buffers should resize")
        );
        assert_eq!(presenter.presented_frames(), 2);

        drop(presenter);
    }

    #[test]
    fn placement_changes_present_once_and_reselecting_the_mode_stays_idle() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let image = decode_image(fixture_path("pixels.png")).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("image should present");
        assert_eq!(presenter.placement_mode(), PlacementMode::Cover);
        assert_eq!(presenter.presented_frames(), 1);

        presenter
            .set_placement_mode(PlacementMode::Fit)
            .expect("fit mode should present");
        assert_eq!(presenter.presented_frames(), 2);
        assert_eq!(presenter.placement_mode(), PlacementMode::Fit);

        presenter
            .set_placement_mode(PlacementMode::Fit)
            .expect("selecting the active mode should be a no-op");
        assert_eq!(presenter.presented_frames(), 2);

        presenter
            .set_placement_mode(PlacementMode::Stretch)
            .expect("stretch mode should present");
        assert_eq!(presenter.presented_frames(), 3);
        drop(presenter);
    }

    #[test]
    fn transform_changes_present_once_and_leave_the_source_file_unchanged() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let path = fixture_path("pixels.png");
        let source_before = std::fs::read(&path).expect("source fixture should be readable");
        let image = decode_image(&path).expect("fixture should decode");
        let mut presenter = D3d11ImagePresenter::new(&device, window.raw_handle(), &image)
            .expect("image should present");

        let crop = NormalizedCrop::new(0.25, 0.0, 0.5, 1.0).expect("crop should be valid");
        let transform = ImageTransform::new((20.0, -10.0), (1.5, 0.75), 17.0, (0.25, 0.8), crop)
            .expect("transform should be valid");
        presenter
            .set_transform(transform)
            .expect("transform should update the geometry");
        assert_eq!(presenter.presented_frames(), 2);
        assert_eq!(presenter.transform(), transform);

        presenter
            .set_transform(transform)
            .expect("reapplying the active transform should be a no-op");
        assert_eq!(presenter.presented_frames(), 2);
        assert_eq!(
            std::fs::read(&path).expect("source fixture should remain readable"),
            source_before
        );
        drop(presenter);
    }
}
