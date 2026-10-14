//! 一次性诊断探针（不参与打包，只用来回答两个问题）：
//! 1. 窗口 `Visible(false)` 之后，`App::update()` 还会不会被调用？
//! 2. 隐藏期间，从**别的线程**发 `ViewportCommand::Visible(true)` 能不能把它唤回来？
//!
//! 跑法：`cargo run --release --example tray_wake_probe`，12 秒后自己退出。

use eframe::egui;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static FRAMES: AtomicUsize = AtomicUsize::new(0);

struct Probe {
    started: Instant,
}

impl eframe::App for Probe {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let n = FRAMES.fetch_add(1, Ordering::SeqCst);
        println!(
            "[probe] update() 被调用 frame={} t={:.2}s",
            n,
            self.started.elapsed().as_secs_f32()
        );
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(format!("tray-wake probe · frame {n}"));
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([320.0, 160.0])
            .with_position([80.0, 80.0])
            .with_title("tray-wake probe"),
        ..Default::default()
    };
    let started = Instant::now();
    eframe::run_native(
        "tray-wake-probe",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let started_at = started;
            std::thread::spawn(move || {
                let mut last_frames = 0usize;
                let mut hidden = false;
                let mut revived = false;
                loop {
                    let t = started_at.elapsed();
                    if t > Duration::from_secs(12) {
                        println!("[probe] 时间到，退出");
                        std::process::exit(0);
                    }
                    if t > Duration::from_secs(3) && !hidden {
                        hidden = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        last_frames = FRAMES.load(Ordering::SeqCst);
                        println!(
                            "[probe] t={:.2} 已发送 Visible(false)，当时 frame={last_frames}",
                            t.as_secs_f32()
                        );
                    }
                    if t > Duration::from_secs(8) && !revived {
                        revived = true;
                        let before = FRAMES.load(Ordering::SeqCst);
                        println!("[probe] t={:.2} 隐藏期间共新增 {} 帧（send_viewport_cmd 有没有唤回来？）",
                                 t.as_secs_f32(), before.saturating_sub(last_frames));
                        // 第二档：绕过 egui，直接从本线程 ShowWindow 主窗口。
                        #[cfg(windows)]
                        unsafe {
                            use windows_sys::Win32::UI::WindowsAndMessaging::{
                                FindWindowW, ShowWindow, SW_SHOW,
                            };
                            let title: Vec<u16> = "tray-wake probe"
                                .encode_utf16()
                                .chain(std::iter::once(0))
                                .collect();
                            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                            let at = Instant::now();
                            let ok = ShowWindow(hwnd, SW_SHOW);
                            println!(
                                "[probe] t={:.2} ShowWindow(hwnd=0x{:x}) 返回 {}，耗时 {:.1}ms",
                                started_at.elapsed().as_secs_f32(),
                                hwnd as usize,
                                ok,
                                at.elapsed().as_secs_f32() * 1000.0
                            );
                        }
                    }
                    if hidden {
                        ctx.request_repaint();
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            });
            Ok(Box::new(Probe { started }))
        }),
    )
}
