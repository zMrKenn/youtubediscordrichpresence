#[cfg(windows)]
use std::env;
#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::path::Path;

// the app mark: violet rounded square with three white "now playing" bars
fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let k = s / 32.0;
    let r = 7.0 * k;
    let bottom = 23.0 * k;
    let bars = [
        (7.0 * k, 12.0 * k, 13.0 * k),
        (13.5 * k, 18.5 * k, 8.0 * k),
        (20.0 * k, 25.0 * k, 12.0 * k),
    ];
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
            let in_bar = bars.iter().any(|&(x0, x1, ytop)| fx >= x0 && fx < x1 && fy >= ytop && fy < bottom);
            if in_bar {
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            } else {
                rgba[idx] = 0x8b;
                rgba[idx + 1] = 0x5c;
                rgba[idx + 2] = 0xf6;
                rgba[idx + 3] = 255;
            }
        }
    }
    rgba
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // build a multi-size .ico and embed it as the exe icon
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
