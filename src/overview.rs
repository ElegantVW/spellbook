use crate::listing::{Entry, Kind};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::SystemTime;

const PREVIEW_LIMIT: usize = 4096;
const IMAGE_FILE_LIMIT: usize = 32 * 1024 * 1024;
const IMAGE_DIMENSION_LIMIT: u32 = 8192;
const IMAGE_ALLOCATION_LIMIT: u64 = 128 * 1024 * 1024;

pub struct Overview {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub size: String,
    pub modified: String,
    pub permissions: String,
    pub preview: Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFormat {
    Plain,
    Markdown,
    Markup,
    Pdf,
}

pub enum Preview {
    Text {
        content: String,
        truncated: bool,
        format: TextFormat,
    },
    Image(ImagePreview),
    Video(ImagePreview),
    Message(String),
}

#[derive(Debug)]
pub struct ImagePreview {
    pub pixels: Vec<Vec<[u8; 3]>>,
}

impl ImagePreview {
    fn from_image(image: image::DynamicImage, columns: u16, rows: u16) -> Self {
        let columns = u32::from(columns.max(1)).min(80);
        let pixel_rows = u32::from(rows.max(1)).saturating_mul(2).min(48);
        let image = image.thumbnail(columns, pixel_rows).to_rgb8();
        let left = (columns - image.width()) / 2;
        let top = (pixel_rows - image.height()) / 2;
        let mut pixels = vec![vec![[0, 0, 0]; columns as usize]; pixel_rows as usize];
        for (x, y, pixel) in image.enumerate_pixels() {
            pixels[(top + y) as usize][(left + x) as usize] = pixel.0;
        }
        Self { pixels }
    }

    pub fn lines(&self) -> Vec<Line<'static>> {
        self.pixels
            .chunks(2)
            .map(|pair| {
                let top = &pair[0];
                let bottom = pair.get(1);
                Line::from(
                    top.iter()
                        .enumerate()
                        .map(|(x, foreground)| {
                            let background = bottom
                                .and_then(|row| row.get(x))
                                .copied()
                                .unwrap_or([0, 0, 0]);
                            Span::styled(
                                "▀",
                                Style::default()
                                    .fg(Color::Rgb(foreground[0], foreground[1], foreground[2]))
                                    .bg(Color::Rgb(background[0], background[1], background[2])),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    }
}

pub fn inspect(entry: &Entry, columns: u16, rows: u16) -> Overview {
    let path = entry.path.display().to_string();
    let metadata = match fs::symlink_metadata(&entry.path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Overview {
                name: entry.name.clone(),
                path,
                kind: kind_label(entry.kind),
                size: "Unavailable".into(),
                modified: "Unavailable".into(),
                permissions: "Unavailable".into(),
                preview: Preview::Message(format!("Could not inspect entry: {error}")),
            };
        }
    };

    let is_link = metadata.file_type().is_symlink();
    let kind = if is_link {
        Kind::Link
    } else if metadata.is_dir() {
        Kind::Dir
    } else {
        entry.kind
    };
    let preview = if is_link {
        Preview::Message("Symbolic link; preview is disabled.".into())
    } else if metadata.is_dir() {
        Preview::Message("Directory; no file preview.".into())
    } else if !metadata.is_file() {
        Preview::Message("Special file; preview is disabled.".into())
    } else {
        read_preview(&entry.path, kind, columns, rows)
    };

    Overview {
        name: entry.name.clone(),
        path,
        kind: kind_label(kind),
        size: if metadata.is_file() {
            crate::listing::format_size(metadata.len())
        } else {
            "—".into()
        },
        modified: metadata
            .modified()
            .map(format_age)
            .unwrap_or_else(|error| format!("Unavailable: {error}")),
        permissions: permissions(&metadata),
        preview,
    }
}

fn read_preview(path: &Path, kind: Kind, columns: u16, rows: u16) -> Preview {
    match kind {
        Kind::Image => return read_image(path, columns, rows, false),
        Kind::Video => return read_image(path, columns, rows, true),
        Kind::Pdf => return read_pdf(path),
        _ => {}
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }

    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) => {
            return Preview::Message(format!("Could not open preview: {error}"));
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return Preview::Message(format!("Could not inspect preview: {error}"));
        }
    };
    if !metadata.is_file() {
        return Preview::Message("Not a regular file; preview is disabled.".into());
    }

    let mut bytes = Vec::with_capacity(PREVIEW_LIMIT + 1);
    if let Err(error) = file
        .by_ref()
        .take((PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        return Preview::Message(format!("Could not read preview: {error}"));
    }
    let truncated = bytes.len() > PREVIEW_LIMIT;
    if bytes.contains(&0) {
        return Preview::Message("Binary file; preview is disabled.".into());
    }
    bytes.truncate(PREVIEW_LIMIT);

    let content = match std::str::from_utf8(&bytes) {
        Ok(content) => content.to_string(),
        Err(error) if truncated && error.error_len().is_none() => {
            match std::str::from_utf8(&bytes[..error.valid_up_to()]) {
                Ok(content) => content.to_string(),
                Err(_) => {
                    return Preview::Message(
                        "Binary or non-UTF-8 file; preview is disabled.".into(),
                    );
                }
            }
        }
        Err(_) => {
            return Preview::Message("Binary or non-UTF-8 file; preview is disabled.".into());
        }
    };

    Preview::Text {
        content: sanitize(&content),
        truncated,
        format: match kind {
            Kind::Markdown => TextFormat::Markdown,
            Kind::Markup => TextFormat::Markup,
            _ => TextFormat::Plain,
        },
    }
}

fn read_image(path: &Path, columns: u16, rows: u16, video: bool) -> Preview {
    if video {
        return match video_frame(path, 0.0, columns, rows) {
            Ok(image) => Preview::Video(image),
            Err(error) => Preview::Message(error),
        };
    }

    let image = {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut file = match options.open(path) {
            Ok(file) => file,
            Err(error) => {
                return Preview::Message(format!("Could not open image preview: {error}"));
            }
        };
        match file.metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Preview::Message("Not a regular file; preview is disabled.".into()),
            Err(error) => {
                return Preview::Message(format!("Could not inspect image preview: {error}"));
            }
        }
        let mut bytes = Vec::new();
        if let Err(error) = file
            .by_ref()
            .take((IMAGE_FILE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
        {
            return Preview::Message(format!("Could not read image preview: {error}"));
        }
        if bytes.len() > IMAGE_FILE_LIMIT {
            return Preview::Message("Image preview is limited to 32 MiB files.".into());
        }
        decode_image(&bytes)
    };

    match image {
        Ok(image) => Preview::Image(ImagePreview::from_image(image, columns, rows)),
        Err(error) => Preview::Message(format!("Could not decode image preview: {error}")),
    }
}

fn decode_image(bytes: &[u8]) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_image_height = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_alloc = Some(IMAGE_ALLOCATION_LIMIT);
    reader.limits(limits);
    reader.decode()
}

pub fn video_frame(
    path: &Path,
    seconds: f64,
    columns: u16,
    rows: u16,
) -> Result<ImagePreview, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect video preview: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Video preview is limited to regular files.".into());
    }
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-ss"])
        .arg(format!("{seconds:.3}"))
        .arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=320:96:force_original_aspect_ratio=decrease",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("Video preview unavailable (ffmpeg): {error}"))?;
    if !output.status.success() {
        return Err("Could not decode a video preview frame.".into());
    }
    let image = decode_image(&output.stdout)
        .map_err(|error| format!("Could not decode video frame: {error}"))?;
    Ok(ImagePreview::from_image(image, columns, rows))
}

fn read_pdf(path: &Path) -> Preview {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {}
        Ok(_) => return Preview::Message("PDF preview is limited to regular files.".into()),
        Err(error) => {
            return Preview::Message(format!("Could not inspect PDF preview: {error}"));
        }
    }
    let mut child = match Command::new("pdftotext")
        .args(["-f", "1", "-l", "1", "-layout"])
        .arg(path)
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Preview::Message(format!("PDF text preview needs pdftotext: {error}"));
        }
    };
    let mut bytes = Vec::with_capacity(PREVIEW_LIMIT + 1);
    let read_result = child
        .stdout
        .take()
        .expect("pdftotext stdout is piped")
        .take((PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut bytes);
    let truncated = bytes.len() > PREVIEW_LIMIT;
    if truncated {
        let _ = child.kill();
    }
    let status = child.wait();
    if let Err(error) = read_result {
        return Preview::Message(format!("Could not read PDF preview: {error}"));
    }
    match status {
        Ok(status) if status.success() || truncated => {}
        Ok(_) => return Preview::Message("Could not extract text from the PDF.".into()),
        Err(error) => return Preview::Message(format!("Could not finish PDF preview: {error}")),
    }
    bytes.truncate(PREVIEW_LIMIT);
    let content = String::from_utf8_lossy(&bytes);
    Preview::Text {
        content: sanitize(&content),
        truncated,
        format: TextFormat::Pdf,
    }
}

pub fn formatted_text(content: &str, format: TextFormat) -> Vec<Line<'static>> {
    let mut fenced_code = false;
    content
        .lines()
        .map(|line| match format {
            TextFormat::Markdown => {
                let trimmed = line.trim_start();
                if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                    fenced_code = !fenced_code;
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Rgb(0x9d, 0x5c, 0x75)),
                    ))
                } else if fenced_code {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Rgb(0x8e, 0xc4, 0xc8)),
                    ))
                } else {
                    markdown_line(line)
                }
            }
            TextFormat::Markup => markup_line(line),
            TextFormat::Plain | TextFormat::Pdf => Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Gray),
            )),
        })
        .collect()
}

fn markdown_line(line: &str) -> Line<'static> {
    let trimmed = line.trim_start();
    let heading = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if heading > 0 && trimmed[heading..].starts_with(' ') {
        return Line::from(Span::styled(
            trimmed[heading..].trim().to_string(),
            Style::default()
                .fg(Color::Rgb(0xf0, 0xd8, 0xa0))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(quote) = trimmed.strip_prefix("> ") {
        return Line::from(Span::styled(
            quote.to_string(),
            Style::default()
                .fg(Color::Rgb(0xc4, 0xb0, 0xd8))
                .add_modifier(Modifier::ITALIC),
        ));
    }
    let bullet_len = if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        Some(2)
    } else {
        trimmed
            .find(". ")
            .filter(|index| *index > 0 && trimmed[..*index].chars().all(|c| c.is_ascii_digit()))
            .map(|index| index + 2)
    };
    if let Some(prefix_len) = bullet_len {
        let indent = line.len() - trimmed.len();
        let mut spans = Vec::new();
        spans.push(Span::raw(" ".repeat(indent)));
        spans.push(Span::styled(
            trimmed[..prefix_len].to_string(),
            Style::default().fg(Color::Rgb(0xe8, 0x79, 0xa0)),
        ));
        spans.extend(inline_markdown(&trimmed[prefix_len..]));
        return Line::from(spans);
    }
    Line::from(inline_markdown(line))
}

fn inline_markdown(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let marker = ["`", "**", "__", "*", "_"]
            .iter()
            .filter_map(|marker| rest.find(marker).map(|index| (index, *marker)))
            .min_by_key(|(index, _)| *index);
        let Some((index, marker)) = marker else {
            spans.push(Span::raw(rest.to_string()));
            break;
        };
        if index > 0 {
            spans.push(Span::raw(rest[..index].to_string()));
        }
        let body_start = index + marker.len();
        if let Some(end) = rest[body_start..].find(marker) {
            let body_end = body_start + end;
            let body = rest[body_start..body_end].to_string();
            let style = if marker == "`" {
                Style::default().fg(Color::Rgb(0x8e, 0xc4, 0xc8))
            } else {
                Style::default()
                    .fg(Color::Rgb(0xf0, 0xd8, 0xa0))
                    .add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(body, style));
            rest = &rest[body_end + marker.len()..];
        } else {
            spans.push(Span::raw(marker.to_string()));
            rest = &rest[body_start..];
        }
    }
    spans
}

fn markup_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut segment = String::new();
    let mut segment_color = None;
    let mut in_tag = false;
    let mut tag_name = false;
    let mut quote = None;
    for character in line.chars() {
        let color = if !in_tag {
            if character == '<' {
                in_tag = true;
                tag_name = true;
                Color::Rgb(0xc4, 0xb0, 0xd8)
            } else {
                Color::Gray
            }
        } else if let Some(q) = quote {
            if character == q {
                quote = None;
            }
            Color::Rgb(0x8e, 0xc4, 0xc8)
        } else if character == '>' {
            in_tag = false;
            Color::Rgb(0xc4, 0xb0, 0xd8)
        } else if character == '\'' || character == '"' {
            quote = Some(character);
            Color::Rgb(0x8e, 0xc4, 0xc8)
        } else if character.is_whitespace() {
            tag_name = false;
            Color::Gray
        } else if tag_name || character == '/' || character == '!' || character == '?' {
            Color::Rgb(0x8e, 0xc4, 0xc8)
        } else if character == '=' {
            Color::Rgb(0x9d, 0x5c, 0x75)
        } else {
            Color::Rgb(0xf0, 0xd8, 0xa0)
        };
        if !segment.is_empty() && segment_color != Some(color) {
            spans.push(Span::styled(
                std::mem::take(&mut segment),
                Style::default().fg(segment_color.unwrap_or(Color::Gray)),
            ));
        }
        segment_color = Some(color);
        segment.push(character);
        if !in_tag {
            tag_name = false;
        }
    }
    if !segment.is_empty() {
        spans.push(Span::styled(
            segment,
            Style::default().fg(segment_color.unwrap_or(Color::Gray)),
        ));
    }
    Line::from(spans)
}

fn sanitize(content: &str) -> String {
    content
        .chars()
        .map(|character| {
            if character == '\t' {
                "    ".into()
            } else if character.is_control() && character != '\n' {
                format!("\\u{{{:x}}}", character as u32)
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Dir => "Directory",
        Kind::Link => "Symbolic link",
        Kind::Exec => "Executable",
        Kind::Image => "Image",
        Kind::Video => "Video",
        Kind::Audio => "Audio",
        Kind::Archive => "Archive",
        Kind::Markdown => "Markdown",
        Kind::Markup => "Markup",
        Kind::Pdf => "PDF",
        Kind::Code => "Code / text",
        Kind::Text => "File",
    }
}

fn format_age(modified: SystemTime) -> String {
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return "in the future".into();
    };
    let seconds = age.as_secs();
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

#[cfg(unix)]
fn permissions(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:04o}", metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn permissions(metadata: &fs::Metadata) -> String {
    if metadata.permissions().readonly() {
        "read-only".into()
    } else {
        "read/write".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::{Listing, Sort};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn inspect_entry(entry: &Entry) -> Overview {
        inspect(entry, 32, 8)
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("spellbook-overview-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn entry(&self, name: &str) -> Entry {
            Listing::open(self.0.clone(), Sort::Name, true)
                .entries
                .into_iter()
                .find(|entry| entry.name == name)
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn previews_text_with_a_hard_byte_limit() {
        let fixture = Fixture::new();
        let contents = format!("{}x", "a".repeat(PREVIEW_LIMIT));
        fs::write(fixture.0.join("large.txt"), contents).unwrap();

        match inspect_entry(&fixture.entry("large.txt")).preview {
            Preview::Text {
                content, truncated, ..
            } => {
                assert_eq!(content.len(), PREVIEW_LIMIT);
                assert!(truncated);
            }
            Preview::Message(message) => panic!("unexpected preview message: {message}"),
            Preview::Image(_) | Preview::Video(_) => panic!("expected text preview"),
        }
    }

    #[test]
    fn previews_empty_text_files() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("empty.txt"), []).unwrap();

        match inspect_entry(&fixture.entry("empty.txt")).preview {
            Preview::Text {
                content, truncated, ..
            } => {
                assert!(content.is_empty());
                assert!(!truncated);
            }
            Preview::Message(message) => panic!("unexpected preview message: {message}"),
            Preview::Image(_) | Preview::Video(_) => panic!("expected text preview"),
        }
    }

    #[test]
    fn escapes_terminal_control_characters_in_text() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("controls.txt"), "hello\u{1b}[31m").unwrap();

        match inspect_entry(&fixture.entry("controls.txt")).preview {
            Preview::Text { content, .. } => {
                assert_eq!(content, "hello\\u{1b}[31m");
            }
            Preview::Message(message) => panic!("unexpected preview message: {message}"),
            Preview::Image(_) | Preview::Video(_) => panic!("expected text preview"),
        }
    }

    #[test]
    fn does_not_preview_binary_or_special_files() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("binary.dat"), [b'a', 0, b'b']).unwrap();
        fs::create_dir(fixture.0.join("folder")).unwrap();

        assert!(matches!(
            inspect_entry(&fixture.entry("binary.dat")).preview,
            Preview::Message(message) if message.contains("Binary")
        ));
        assert!(matches!(
            inspect_entry(&fixture.entry("folder")).preview,
            Preview::Message(message) if message.contains("Directory")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symbolic_links_for_preview() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        fs::write(fixture.0.join("secret.txt"), "not previewed").unwrap();
        symlink(fixture.0.join("secret.txt"), fixture.0.join("link.txt")).unwrap();

        assert!(matches!(
            inspect_entry(&fixture.entry("link.txt")).preview,
            Preview::Message(message) if message.contains("Symbolic link")
        ));
    }

    #[test]
    fn missing_file_errors_are_shown() {
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing.txt");
        assert!(matches!(
            read_preview(&missing, Kind::Text, 32, 8),
            Preview::Message(message) if message.contains("Could not open preview")
        ));
    }

    #[test]
    fn decodes_images_to_bounded_terminal_pixels() {
        let fixture = Fixture::new();
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            160,
            80,
            image::Rgb([240, 20, 90]),
        ));
        image
            .save(fixture.0.join("picture.png"))
            .expect("save image fixture");
        match inspect_entry(&fixture.entry("picture.png")).preview {
            Preview::Image(preview) => {
                assert_eq!(preview.pixels.len(), 16);
                assert_eq!(preview.pixels[0].len(), 32);
                assert_eq!(preview.pixels[7][15], [240, 20, 90]);
            }
            Preview::Message(message) => panic!("unexpected image preview message: {message}"),
            _ => panic!("expected image preview"),
        }
    }

    #[test]
    fn formats_markdown_and_markup_with_styles() {
        let markdown = formatted_text("# Heading\n**bold** and `code`", TextFormat::Markdown);
        assert_eq!(markdown.len(), 2);
        assert!(markdown[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(markdown[1]
            .spans
            .iter()
            .any(|span| span.style.fg == Some(Color::Rgb(0x8e, 0xc4, 0xc8))));

        let markup = formatted_text("<tag attr=\"value\">text</tag>", TextFormat::Markup);
        let colors: Vec<_> = markup[0]
            .spans
            .iter()
            .filter_map(|span| span.style.fg)
            .collect();
        assert!(colors.contains(&Color::Rgb(0xc4, 0xb0, 0xd8)));
        assert!(colors.contains(&Color::Rgb(0x8e, 0xc4, 0xc8)));
        assert!(colors.contains(&Color::Rgb(0xf0, 0xd8, 0xa0)));
    }

    #[test]
    fn extracts_bounded_text_from_pdf_first_page_when_tool_is_available() {
        if Command::new("pdftotext").arg("-v").output().is_err() {
            return;
        }
        let fixture = Fixture::new();
        let path = fixture.0.join("sample.pdf");
        let stream = b"BT /F1 18 Tf 20 100 Td (PDF preview works) Tj ET";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", stream.len()).as_bytes(),
                stream,
                b"\nendstream",
            ]
            .concat(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0usize];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        fs::write(&path, pdf).unwrap();
        match read_pdf(&path) {
            Preview::Text {
                content,
                truncated: false,
                format: TextFormat::Pdf,
            } => assert!(content.contains("PDF preview works"), "got: {content:?}"),
            Preview::Message(message) => panic!("unexpected PDF preview message: {message}"),
            _ => panic!("unexpected PDF preview result"),
        }
    }

    #[test]
    fn rejects_video_symlinks_before_invoking_ffmpeg() {
        let fixture = Fixture::new();
        let target = fixture.0.join("target.mp4");
        let link = fixture.0.join("link.mp4");
        fs::write(&target, b"not a real video").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(unix)]
        assert!(video_frame(&link, 0.0, 32, 8)
            .unwrap_err()
            .contains("regular files"));
    }
}
