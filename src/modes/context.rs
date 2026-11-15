use super::Message;

pub const DEFAULT_CONTEXT_BYTES: usize = 24_000;

pub fn truncate_utf8(s: &str, cap: usize) -> &str {
    let mut end = cap.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn bounded(messages: &[Message], cap: usize) -> String {
    let mut lines = Vec::new();
    let mut used = 0;
    for message in messages.iter().rev() {
        let line = format!("{}: {}\n", message.author, message.text);
        if line.len() > cap - used {
            if lines.is_empty() {
                lines.push(truncate_utf8(&line, cap).to_owned());
            }
            break;
        }
        used += line.len();
        lines.push(line);
    }
    lines.reverse();
    lines.concat()
}
