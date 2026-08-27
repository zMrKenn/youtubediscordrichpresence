#[cfg(windows)]
use std::env;
#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::path::Path;

fn sign(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    (px - bx) * (ay - by) - (ax - bx) * (py - by)
}

fn in_triangle(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> bool {
    let d1 = sign(px, py, ax, ay, bx, by);
    let d2 = sign(px, py, bx, by, cx, cy);
    let d3 = sign(px, py, cx, cy, ax, ay);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

// The app icon: red rounded square, white play triangle. Same thing the tray uses.
fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let k = s / 32.0;
    let (ax, ay, bx, by, cx, cy) = (12.0 * k, 9.0 * k, 12.0 * k, 23.0 * k, 23.0 * k, 16.0 * k);
    let r = 6.0 * k;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let fx = x as f32;
            let fy = y as f32;
            let mut transparent = false;
            let corners = [(r, r), (s - r, r), (r, s - r), (s - r, s - r)];
            for (cxr, cyr) in corners {
                let ox = (fx < r && cxr == r) || (fx > s - r && cxr == s - r);
                let oy = (fy < r && cyr == r) || (fy > s - r && cyr == s - r);
                if ox && oy {
                    let dx = fx + 0.5 - cxr;
                    let dy = fy + 0.5 - cyr;
                    if dx * dx + dy * dy > r * r {
                        transparent = true;
                    }
                }
            }
            if transparent {
                continue;
            }
            if in_triangle(fx, fy, ax, ay, bx, by, cx, cy) {
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            } else {
                rgba[idx] = 255;
                rgba[idx + 1] = 0;
                rgba[idx + 2] = 0;
                rgba[idx + 3] = 255;
            }
        }
    }
    rgba
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // Generate a multi-size .ico and embed it as the exe's file icon (Windows).
    #[cfg(windows)]
    {
        let out_dir = env::var("OUT_DIR").unwrap();
        let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
        for sz in [16u32, 24, 32, 48, 64, 128, 256] {
            let image = ico::IconImage::from_rgba_data(sz, sz, icon_rgba(sz));
            dir.add_entry(ico::IconDirEntry::encode(&image).unwrap());
        }
        let ico_path = Path::new(&out_dir).join("app.ico");
        let file = fs::File::create(&ico_path).unwrap();
        dir.write(file).unwrap();

        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        if let Err(e) = res.compile() {
            println!("cargo:warning=exe icon embedding skipped: {e}");
        }
    }

    let _ = icon_rgba;
}
