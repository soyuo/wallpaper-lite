use std::slice;

use wallpaper_media::DecodedImage;
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
                D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT, ID3D11Buffer, ID3D11DepthStencilView,
                ID3D11DeviceContext, ID3D11PixelShader, ID3D11RasterizerState,
                ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
                ID3D11Texture2D, ID3D11VertexShader,
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
    D3d11Device, PlacementDimensions, PlacementMode, RendererError, calculate_image_placement,
};

const VERTEX_SHADER: &str = r#"
struct VertexOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

cbuffer PlacementData : register(b0) {
    float4 destination_rect;
    float4 source_uv_rect;
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
    output.position = float4(
        lerp(destination_rect.x, destination_rect.z, coordinates[vertex_id].x),
        lerp(destination_rect.y, destination_rect.w, coordinates[vertex_id].y),
        0.0,
        1.0
    );
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

float4 ps_main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    return source_image.Sample(image_sampler, uv);
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
    presented_frames: u64,
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
        let hwnd = hwnd_from_raw(window_handle);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(RendererError::WindowUnavailable);
        }
        let (width, height) = client_size(hwnd)?;
        let source_dimensions = PlacementDimensions::new(image.width(), image.height())?;
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

        let (image_texture, image_view) = create_image_texture(device, image)?;
        let placement_buffer =
            create_placement_buffer(device, source_dimensions, output_dimensions, placement_mode)?;
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
            presented_frames: 0,
        };
        presenter.create_render_target()?;
        presenter.render_current_image()?;
        Ok(presenter)
    }

    /// Replace the current GPU texture and present the new image once.
    pub fn set_image(&mut self, image: &DecodedImage) -> Result<(), RendererError> {
        let (texture, view) = create_image_texture(self.device, image)?;
        let source_dimensions = PlacementDimensions::new(image.width(), image.height())?;
        let placement_buffer = create_placement_buffer(
            self.device,
            source_dimensions,
            PlacementDimensions::new(self.width, self.height)?,
            self.placement_mode,
        )?;
        self.unbind_resources();
        self.image_texture = texture;
        self.image_view = view;
        self.source_dimensions = source_dimensions;
        self.placement_buffer = placement_buffer;
        self.render_current_image()
    }

    /// Change scale and crop behavior, drawing one frame only when the mode changes.
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
        )?;
        self.placement_buffer = placement_buffer;
        self.placement_mode = placement_mode;
        self.render_current_image()
    }

    pub const fn placement_mode(&self) -> PlacementMode {
        self.placement_mode
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
        let placement_buffers = [Some(self.placement_buffer.clone())];
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
            context.PSSetShaderResources(0, Some(&image_views));
            context.PSSetSamplers(0, Some(&samplers));
            context.Draw(6, 0);
            context.PSSetShaderResources(0, Some(&[None]));
            context.VSSetConstantBuffers(0, Some(&[None]));
            context.OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }

        unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|error| RendererError::Present {
                hresult: error.code().0,
            })?;
        self.presented_frames = self.presented_frames.saturating_add(1);
        Ok(())
    }

    fn unbind_resources(&self) {
        unsafe {
            self.device
                .immediate_context()
                .PSSetShaderResources(0, Some(&[None]));
            self.device
                .immediate_context()
                .VSSetConstantBuffers(0, Some(&[None]));
            self.device
                .immediate_context()
                .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
    }
}

impl Drop for D3d11ImagePresenter<'_> {
    fn drop(&mut self) {
        self.unbind_resources();
    }
}

fn create_image_texture(
    device: &D3d11Device,
    image: &DecodedImage,
) -> Result<(ID3D11Texture2D, ID3D11ShaderResourceView), RendererError> {
    let byte_len = image
        .stride()
        .checked_mul(image.height())
        .ok_or(RendererError::ImageTextureCreation { hresult: 0 })?;
    let subresource = D3D11_SUBRESOURCE_DATA {
        pSysMem: image.rgba8().as_ptr().cast(),
        SysMemPitch: image.stride(),
        SysMemSlicePitch: byte_len,
    };
    let description = D3D11_TEXTURE2D_DESC {
        Width: image.width(),
        Height: image.height(),
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
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
    destination_clip: [f32; 4],
    source_uv: [f32; 4],
}

fn create_placement_buffer(
    device: &D3d11Device,
    source: PlacementDimensions,
    output: PlacementDimensions,
    mode: PlacementMode,
) -> Result<ID3D11Buffer, RendererError> {
    let placement = calculate_image_placement(source, output, mode);
    let destination = placement.destination();
    let source_uv = placement.source_uv();
    let constants = PlacementConstants {
        destination_clip: [
            destination.left() * 2.0 - 1.0,
            1.0 - destination.top() * 2.0,
            destination.right() * 2.0 - 1.0,
            1.0 - destination.bottom() * 2.0,
        ],
        source_uv: [
            source_uv.left(),
            source_uv.top(),
            source_uv.right(),
            source_uv.bottom(),
        ],
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
    use wallpaper_media::decode_image;
    use windows::Win32::UI::WindowsAndMessaging::SetWindowPos;

    use super::{D3d11ImagePresenter, hwnd_from_raw};
    use crate::D3d11Device;
    use crate::PlacementMode;
    use windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("media")
            .join("tests")
            .join("fixtures")
            .join(name)
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
}
