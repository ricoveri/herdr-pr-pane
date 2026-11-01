//! Turn GitHub-flavoured markdown/HTML comment bodies into plain wrapped lines.

/// Strip HTML tags, `<details>` chrome, images and HTML comments; keep text.
pub fn clean(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let mut in_fence = false;
    let mut line = String::new();
    let flush = |line: &mut String, out: &mut String, in_fence: &mut bool| {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            *in_fence = !*in_fence;
        } else if !t.is_empty() || *in_fence {
            out.push_str(if *in_fence { "  " } else { "" });
            out.push_str(t);
            out.push('\n');
        } else if !out.ends_with("\n\n") && !out.is_empty() {
            out.push('\n');
        }
        line.clear();
    };
    while let Some(c) = chars.next() {
        match c {
            '\n' => flush(&mut line, &mut out, &mut in_fence),
            '<' if !in_fence => {
                let mut tag = String::new();
                for t in chars.by_ref() {
                    if t == '>' {
                        break;
                    }
                    tag.push(t);
                }
                if tag.starts_with("!--") {
                    continue;
                }
                let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("");
                if matches!(name, "br" | "p" | "div" | "summary" | "li") {
                    flush(&mut line, &mut out, &mut in_fence);
                }
            }
            _ => line.push(c),
        }
    }
    flush(&mut line, &mut out, &mut in_fence);
    // Markdown noise: images become their alt text, headings/emphasis lose markers.
    out.lines()
        .map(|l| {
            let l = l.trim_start_matches('#').trim_start();
            l.replace("**", "").replace("__", "").replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Greedy word wrap on display columns (chars as a good-enough proxy).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let indent: String = raw.chars().take_while(|c| *c == ' ').collect();
        let mut cur = indent.clone();
        for word in raw.split_whitespace() {
            let w = word.chars().count();
            if cur.chars().count() + w + 1 > width && cur.trim().len() > 0 {
                lines.push(std::mem::replace(&mut cur, indent.clone()));
            }
            // Hard-split words longer than a row (long URLs).
            let mut word = word.to_owned();
            while word.chars().count() > width {
                let head: String = word.chars().take(width).collect();
                word = word.chars().skip(width).collect();
                if !cur.trim().is_empty() {
                    lines.push(std::mem::replace(&mut cur, indent.clone()));
                }
                lines.push(head);
            }
            if !cur.trim().is_empty() {
                cur.push(' ');
            }
            cur.push_str(&word);
        }
        if !cur.trim().is_empty() {
            lines.push(cur);
        }
    }
    lines
}

/// `2026-10-30T15:06:31Z` -> `1d ago` style.
pub fn ago(iso: &str) -> String {
    // Avoid a chrono dependency: parse the fixed RFC3339 shape.
    let p = |a: usize, b: usize| iso.get(a..b).and_then(|s| s.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi)) = (p(0, 4), p(5, 7), p(8, 10), p(11, 13), p(14, 16)) else {
        return iso.to_owned();
    };
    let days_from_civil = |y: i64, m: i64, d: i64| {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    };
    let then = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(then);
    let s = (now - then).max(0);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}
