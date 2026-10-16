use super::*;

struct HiddenWindow(HWND);
impl Drop for HiddenWindow {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0) };
    }
}

#[test]
fn centers_on_secondary_monitors_and_respects_work_area() {
    for (work, size, expected) in [
        ((0, 0, 1920, 1040), (700, 540), (610, 250)),
        ((1920, 0, 3840, 1040), (700, 540), (2530, 250)),
        ((-1920, 0, 0, 1040), (700, 540), (-1310, 250)),
        ((0, -1080, 1920, -40), (700, 540), (610, -830)),
        ((48, 40, 1920, 1080), (700, 540), (634, 290)),
        // Same logical window at 150% and 200% on a 4K work area.
        ((1920, 0, 5760, 2100), (1050, 810), (3315, 645)),
        ((1920, 0, 5760, 2080), (1400, 1080), (3140, 500)),
        // Oversized windows keep their top-left controls accessible.
        ((-800, 0, 0, 560), (1000, 700), (-800, 0)),
    ] {
        let (left, top, right, bottom) = work;
        assert_eq!(
            centered_origin(
                RECT {
                    left,
                    top,
                    right,
                    bottom
                },
                size.0,
                size.1
            ),
            expected,
            "work={work:?}, size={size:?}",
        );
    }
}

#[test]
fn launch_coordinates_round_trip_including_negative_monitors() {
    for value in ["0,0", "-960,540", "960,-540", "-960,-540"] {
        assert_eq!(value.parse::<LaunchMonitor>().unwrap().to_string(), value);
    }
    for value in ["", "1", "1,2,3", "x,1", "1,y", "2147483648,0"] {
        assert!(value.parse::<LaunchMonitor>().is_err(), "{value}");
    }
}

#[test]
fn centers_a_real_hidden_window_without_resizing_or_activating_it() {
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Installway placement test"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            700,
            540,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let _window = HiddenWindow(hwnd);
        let foreground = GetForegroundWindow();
        let mut before = RECT::default();
        GetWindowRect(hwnd, &mut before).unwrap();
        center_on(hwnd, LaunchMonitor::default());
        let mut after = RECT::default();
        GetWindowRect(hwnd, &mut after).unwrap();
        let mut info = monitor_info();
        assert!(
            GetMonitorInfoW(
                MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY),
                &mut info
            )
            .as_bool()
        );
        assert_eq!(
            (after.right - after.left, after.bottom - after.top),
            (before.right - before.left, before.bottom - before.top)
        );
        assert_eq!(
            (after.left, after.top),
            centered_origin(info.rcWork, 700, 540)
        );
        assert_eq!(GetForegroundWindow(), foreground);
        // Moving a window later must remain possible (centering is initial only).
        SetWindowPos(
            hwnd,
            None,
            20,
            30,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
        GetWindowRect(hwnd, &mut after).unwrap();
        assert_eq!((after.left, after.top), (20, 30));
    }
}

#[test]
fn recenters_after_a_synchronous_dpi_resize_during_the_first_move() {
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, RegisterClassW,
        SendMessageW, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_DPICHANGED, WM_WINDOWPOSCHANGED,
        WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::{PCWSTR, w};

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if msg == WM_DPICHANGED {
                crate::win32::follow_dpi_change(hwnd, w, l);
                return LRESULT(0);
            }
            if msg == WM_WINDOWPOSCHANGED && GetWindowLongPtrW(hwnd, GWLP_USERDATA) == 1 {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let mut rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rect);
                rect.right = rect.left + 1050;
                rect.bottom = rect.top + 810;
                SendMessageW(
                    hwnd,
                    WM_DPICHANGED,
                    Some(WPARAM(144 | (144 << 16))),
                    Some(LPARAM(&rect as *const RECT as isize)),
                );
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
    }

    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(PCWSTR::null()).unwrap().0);
        let class = w!("InstallwayDpiPlacementTest");
        assert_ne!(
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            }),
            0
        );
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            700,
            540,
            None,
            None,
            Some(instance),
            None,
        )
        .unwrap();
        let _window = HiddenWindow(hwnd);
        let mut info = monitor_info();
        assert!(
            GetMonitorInfoW(
                MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY),
                &mut info
            )
            .as_bool()
        );
        // Offset from the destination so the first centering move fires.
        let (x, y) = centered_origin(info.rcWork, 700, 540);
        SetWindowPos(
            hwnd,
            None,
            x + 17,
            y + 23,
            700,
            540,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
        center_on(hwnd, LaunchMonitor::default());
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).unwrap();
        assert_eq!(
            (rect.right - rect.left, rect.bottom - rect.top),
            (1050, 810)
        );
        assert_eq!(
            (rect.left, rect.top),
            centered_origin(info.rcWork, 1050, 810)
        );
    }
}
