//! 一次性探针（不参与打包，跑完可删）：把"窗口收进托盘之后 UI 线程为什么烧满一个核"
//! 在 100 行里复现出来，并把修法一条条量清楚。
//!
//! 跑法（14 秒自己退出；窗口一律建在屏幕外 + `with_active(false)`，不打扰桌面）：
//!   cargo run --release --example park_probe --                      # 屏幕外且可见：基线
//!   ... -- hide-later                                                # 起来 1 秒后 Visible(false)，什么都不登记
//!   ... -- hide-later beat                                           # 同上 + 每帧 request_repaint_after(1s)  → 复现空转
//!   ... -- hide-later beat beatskip                                  # 发出隐藏那一帧就不登记 → 应当 0 CPU
//!   ... -- hide-later poke                                           # 隐藏后 3 秒从别的线程 request_repaint() 一次
//!   ... -- hide-later beat beatskip poke                             # 修好之后再来一次托盘式唤醒
//! 判据：`scripts/cpu_by_name.ps1` 的 ms/s（一核＝1000）+ 日志里的帧数。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

static FRAMES: AtomicUsize = AtomicUsize::new(0);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

struct Probe {
    hide_at: Option<f32>,
    hidden_done: bool,
    beat: bool,
    beatskip: bool,
}

/// 窗口现在收得到 WM_PAINT 吗？只用本线程的非阻塞查询，绝不碰要往别的线程发消息的 API。
#[cfg(windows)]
fn paintable() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, IsWindowVisible};
    unsafe {
        let title: Vec<u16> = "park probe"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0
    }
}

#[cfg(not(windows))]
fn paintable() -> bool {
    true
}

impl eframe::App for Probe {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let n = FRAMES.fetch_add(1, Ordering::SeqCst);
        let t = START.elapsed().as_secs_f32();
        let can_paint = paintable();
        println!("[ui] frame={n} t={t:.2} paintable={can_paint}");

        let just_hid = match self.hide_at {
            Some(at) if !self.hidden_done && t >= at => {
                self.hidden_done = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                println!("[ui] t={t:.2} 已发出 Visible(false)");
                true
            }
            _ => false,
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(format!("park_probe frame {n}"));
        });

        if self.beat {
            if self.beatskip && (just_hid || !can_paint) {
                println!("[ui] t={t:.2} 刚隐藏/画不了 -> 不登记延后重绘");
            } else {
                ctx.request_repaint_after(Duration::from_millis(1000));
            }
        }

        if t >= 14.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |name: &str| args.iter().any(|a| a == name);
    let hide_later = has("hide-later");
    let beat = has("beat");
    let beatskip = has("beatskip");
    let poke = has("poke");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([300.0, 120.0])
            .with_position([-30000.0, -30000.0])
            .with_title("park probe")
            .with_decorations(false)
            .with_active(false),
        ..Default::default()
    };
    println!(
        "[main] mode={} beat={beat} beatskip={beatskip} poke={poke}",
        if hide_later {
            "hide-later"
        } else {
            "offscreen"
        }
    );
    eframe::run_native(
        "park-probe",
        options,
        Box::new(move |cc| {
            if poke {
                let ctx = cc.egui_ctx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(3));
                    println!("[poke] t=3.00 别的线程 ctx.request_repaint()");
                    ctx.request_repaint();
                });
            }
            Ok(Box::new(Probe {
                hide_at: if hide_later { Some(1.0) } else { None },
                hidden_done: false,
                beat,
                beatskip,
            }))
        }),
    )
}
