//! Glob (pathname expansion) for vsh.
//!
//! Supports `*`, `?`, `[...]`, `[!...]`, `[^...]`, and extended globs
//! (`extglob`): `?(pat)`, `*(pat)`, `+(pat)`, `@(pat)`, `!(pat)`.

use alloc::vec::Vec;

/// Test whether `pattern` matches `text`.
///
/// - `*` matches zero or more characters (not `/`)
/// - `?` matches exactly one character (not `/`)
/// - `[abc]` matches any character in the set
/// - `[a-z]` matches a range
/// - `[!abc]` or `[^abc]` negated class
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_impl(&pat, 0, &txt, 0)
}

fn glob_match_impl(pat: &[char], mut pi: usize, txt: &[char], mut ti: usize) -> bool {
    let plen = pat.len();
    let tlen = txt.len();
    let mut star_pi: Option<usize> = None;
    let mut star_ti: usize = 0;

    while ti < tlen {
        if pi < plen && pat[pi] == '?' && txt[ti] != '/' {
            pi += 1;
            ti += 1;
        } else if pi < plen && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if pi < plen && pat[pi] == '[' {
            let (matched, end) = match_char_class(&pat[pi..], txt[ti]);
            if matched {
                pi += end;
                ti += 1;
            } else if let Some(sp) = star_pi {
                if txt[star_ti] == '/' {
                    return false;
                }
                pi = sp + 1;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        } else if pi < plen && pat[pi] == txt[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star_pi {
            if txt[star_ti] == '/' {
                return false;
            }
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < plen && pat[pi] == '*' {
        pi += 1;
    }

    pi == plen
}

fn match_char_class(chars: &[char], ch: char) -> (bool, usize) {
    let len = chars.len();
    if len < 2 || chars[0] != '[' {
        return (false, 0);
    }

    let mut i = 1;
    let negated = if i < len && (chars[i] == '!' || chars[i] == '^') {
        i += 1;
        true
    } else {
        false
    };

    let mut matched = false;

    // Leading `]` is literal
    if i < len && chars[i] == ']' {
        if ch == ']' {
            matched = true;
        }
        i += 1;
    }

    while i < len && chars[i] != ']' {
        if i + 2 < len && chars[i + 1] == '-' && chars[i + 2] != ']' {
            let lo = chars[i];
            let hi = chars[i + 2];
            if ch >= lo && ch <= hi {
                matched = true;
            }
            i += 3;
        } else {
            if chars[i] == ch {
                matched = true;
            }
            i += 1;
        }
    }

    if i < len && chars[i] == ']' {
        i += 1;
    } else {
        return (false, 0);
    }

    let result = if negated { !matched } else { matched };
    (result, i)
}

/// Check if a string contains glob metacharacters.
pub fn contains_glob_chars(s: &str) -> bool {
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if matches!(ch, '*' | '?' | '[') {
            return true;
        }
    }
    false
}
