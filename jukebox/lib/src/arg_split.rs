#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum ParseState {
    Plain = b' ' as isize,
    InQuotes = b'\'' as isize,
    InDoubleQuotes = b'"' as isize,
}
use ParseState::*;

pub fn quoted_parse(s: &str) -> Vec<&str> {
    let s = s.trim();
    let mut start = 0;
    let mut vec = Vec::with_capacity(!s.is_empty() as usize);
    loop {
        let state = match s.get(start..(start + 1)) {
            Some("\"") => InDoubleQuotes,
            Some("'") => InQuotes,
            Some(_) | None => Plain,
        };
        start += (state != Plain) as usize;
        let end = match s[start..].find(|c| c == state as u8 as char) {
            Some(end) => end + start,
            None => s.len(),
        };
        if end - start > 0 {
            vec.push(&s[start..end]);
        }
        start = end + 1;
        if start >= s.len() {
            break;
        }
    }
    vec
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn simple() {
        let s = "a";
        assert_eq!(quoted_parse(s), vec!["a"])
    }

    #[test]
    fn two_words() {
        let s = "a b";
        assert_eq!(quoted_parse(s), vec!["a", "b"])
    }

    #[test]
    fn extra_space() {
        let s = "a  b";
        assert_eq!(quoted_parse(s), vec!["a", "b"])
    }

    #[test]
    fn quotes() {
        let s = "queue 'ola amigos'";
        assert_eq!(quoted_parse(s), vec!["queue", "ola amigos"])
    }

    #[test]
    fn double_quotes_in_quotes() {
        let s = "queue 'ola \"isto nao conta\" amigos'";
        assert_eq!(
            quoted_parse(s),
            vec!["queue", "ola \"isto nao conta\" amigos"]
        )
    }

    #[test]
    fn double_quotes() {
        let s = r#"bem "agora com aspas" lets go"#;
        assert_eq!(
            quoted_parse(s),
            vec!["bem", "agora com aspas", "lets", "go"]
        )
    }

    #[test]
    fn quotes_in_double_quotes() {
        let s = r#"bem "agora 'isto nao conta lol' com aspas" lets go"#;
        assert_eq!(
            quoted_parse(s),
            vec!["bem", "agora 'isto nao conta lol' com aspas", "lets", "go"]
        )
    }

    #[test]
    fn permissive() {
        let s = "simple 'compound arg not terminated";
        assert_eq!(
            quoted_parse(s),
            vec!["simple", "compound arg not terminated"],
        )
    }
}
