use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::ptr::null_mut;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{GetLastError, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORENUMPROC, MONITORINFO,
    MONITORINFOEXW,
};
use windows::Win32::UI::Shell::GetScaleFactorForMonitor;
use windows::core::BOOL;

use crate::{
    DisplayError, DisplayId, DisplayInfo, DisplayRect, DisplaySnapshot, DisplayStage, RefreshRate,
};

const MAX_TOPOLOGY_RETRIES: usize = 3;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const MONITORINFOF_PRIMARY: u32 = 1;

struct ActiveTarget {
    source_device_name: String,
    id: DisplayId,
    friendly_name: String,
    refresh_rate: Option<RefreshRate>,
}

struct MonitorRecord {
    handle: HMONITOR,
}

pub(super) fn enumerate_displays() -> Result<DisplaySnapshot, DisplayError> {
    let targets = query_active_targets()?;
    if targets.is_empty() {
        return Ok(DisplaySnapshot::default());
    }

    let monitors = enumerate_monitor_handles()?;
    let mut targets_by_source = HashMap::<String, Vec<ActiveTarget>>::new();
    for target in targets {
        targets_by_source
            .entry(normalize_device_name(&target.source_device_name))
            .or_default()
            .push(target);
    }

    let expected_ids = targets_by_source
        .values()
        .flatten()
        .map(|target| target.id.clone())
        .collect::<HashSet<_>>();
    let mut displays = Vec::new();
    let mut matched_sources = HashSet::new();
    for monitor in monitors {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let monitor_info = unsafe {
            GetMonitorInfoW(
                monitor.handle,
                (&mut info.monitorInfo as *mut MONITORINFO).cast(),
            )
        };
        if !monitor_info.as_bool() {
            return Err(DisplayError::WindowsApi {
                stage: DisplayStage::MonitorInformation,
                code: unsafe { GetLastError() }.0 as i32,
            });
        }

        let device_name = utf16_string(&info.szDevice);
        let source_key = normalize_device_name(&device_name);
        let Some(active_targets) = targets_by_source.get(&source_key) else {
            // EnumDisplayMonitors can return pseudo-monitors that are not active
            // desktop targets. Only expose targets returned by QueryDisplayConfig.
            continue;
        };
        // Clone paths can share one GDI source and therefore one logical
        // desktop monitor record. Emit their distinct targets once.
        if !matched_sources.insert(source_key) {
            continue;
        }

        let scale = unsafe { GetScaleFactorForMonitor(monitor.handle) }.map_err(|error| {
            DisplayError::WindowsApi {
                stage: DisplayStage::MonitorScaleFactor,
                code: error.code().0,
            }
        })?;
        let scale_percent = u32::try_from(scale.0).map_err(|_| DisplayError::InvalidDpiScale)?;
        let bounds = info.monitorInfo.rcMonitor;
        let bounds = DisplayRect::new(bounds.left, bounds.top, bounds.right, bounds.bottom)?;
        let primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;

        for target in active_targets {
            let name = if target.friendly_name.is_empty() {
                device_name.as_str()
            } else {
                target.friendly_name.as_str()
            };
            displays.push(DisplayInfo::new(
                target.id.clone(),
                name,
                device_name.as_str(),
                bounds,
                scale_percent,
                target.refresh_rate,
                primary,
            )?);
        }
    }

    let observed_ids = displays
        .iter()
        .map(|display| display.id().clone())
        .collect::<HashSet<_>>();
    if observed_ids != expected_ids {
        return Err(DisplayError::DisplayTopologyChanged);
    }

    DisplaySnapshot::new(displays)
}

fn query_active_targets() -> Result<Vec<ActiveTarget>, DisplayError> {
    for _ in 0..MAX_TOPOLOGY_RETRIES {
        let flags = QDC_ONLY_ACTIVE_PATHS;
        let mut path_count = 0_u32;
        let mut mode_count = 0_u32;
        let buffer_status =
            unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) };
        if buffer_status.0 != 0 {
            return Err(DisplayError::WindowsApi {
                stage: DisplayStage::DisplayConfigBufferSizes,
                code: buffer_status.0 as i32,
            });
        }

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![zeroed_mode_info(); mode_count as usize];
        let mut actual_path_count = path_count;
        let mut actual_mode_count = mode_count;
        let status = unsafe {
            QueryDisplayConfig(
                flags,
                &mut actual_path_count,
                if paths.is_empty() {
                    null_mut()
                } else {
                    paths.as_mut_ptr()
                },
                &mut actual_mode_count,
                if modes.is_empty() {
                    null_mut()
                } else {
                    modes.as_mut_ptr()
                },
                None,
            )
        };

        if status.0 == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status.0 != 0 {
            return Err(DisplayError::WindowsApi {
                stage: DisplayStage::DisplayConfigPaths,
                code: status.0 as i32,
            });
        }
        if actual_path_count as usize > paths.len() {
            continue;
        }

        paths.truncate(actual_path_count as usize);
        return paths.into_iter().map(active_target).collect();
    }

    Err(DisplayError::DisplayTopologyChanged)
}

fn active_target(path: DISPLAYCONFIG_PATH_INFO) -> Result<ActiveTarget, DisplayError> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..Default::default()
    };
    let source_status = unsafe {
        DisplayConfigGetDeviceInfo(
            (&mut source.header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER).cast(),
        )
    };
    if source_status != 0 {
        return Err(DisplayError::WindowsApi {
            stage: DisplayStage::DisplayConfigSourceName,
            code: source_status,
        });
    }

    let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };
    let target_status = unsafe {
        DisplayConfigGetDeviceInfo(
            (&mut target.header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER).cast(),
        )
    };
    if target_status != 0 {
        return Err(DisplayError::WindowsApi {
            stage: DisplayStage::DisplayConfigTargetName,
            code: target_status,
        });
    }

    let source_device_name = utf16_string(&source.viewGdiDeviceName);
    let id = DisplayId::from_device_path(utf16_string(&target.monitorDevicePath))?;
    let friendly_name = utf16_string(&target.monitorFriendlyDeviceName);
    let rate = path.targetInfo.refreshRate;
    let refresh_rate = if rate.Numerator == 0 || rate.Denominator == 0 {
        None
    } else {
        Some(RefreshRate::new(rate.Numerator, rate.Denominator)?)
    };

    Ok(ActiveTarget {
        source_device_name,
        id,
        friendly_name,
        refresh_rate,
    })
}

fn enumerate_monitor_handles() -> Result<Vec<MonitorRecord>, DisplayError> {
    let mut monitors = Vec::<MonitorRecord>::new();
    let data = LPARAM((&mut monitors as *mut Vec<MonitorRecord>) as isize);
    let result = unsafe { EnumDisplayMonitors(None, None, Some(collect_monitor), data) };
    if !result.as_bool() {
        return Err(DisplayError::WindowsApi {
            stage: DisplayStage::MonitorEnumeration,
            code: unsafe { GetLastError() }.0 as i32,
        });
    }

    Ok(monitors)
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<MonitorRecord>) };
    monitors.push(MonitorRecord { handle: monitor });
    BOOL(1)
}

fn normalize_device_name(device_name: &str) -> String {
    device_name.to_ascii_uppercase()
}

fn utf16_string(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

fn zeroed_mode_info() -> DISPLAYCONFIG_MODE_INFO {
    // QueryDisplayConfig fills mode entries for the active paths before use.
    unsafe { std::mem::zeroed() }
}

const _: MONITORENUMPROC = Some(collect_monitor);
