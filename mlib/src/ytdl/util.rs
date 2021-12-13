use once_cell::sync::Lazy;
use regex::Regex;

static ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(v=|youtu.be/)(?P<id>[A-Za-z0-9_\-]{11})").unwrap());

pub fn extract_id(s: &str) -> Option<&str> {
    Some(ID.captures(s)?.name("id").unwrap().as_str())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn long_url() {
        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
        assert_eq!("jNQXAC9IVRw", extract_id(url).unwrap());
    }

    #[test]
    fn short_url() {
        let url = "https://youtu.be/jNQXAC9IVRw";
        assert_eq!("jNQXAC9IVRw", extract_id(url).unwrap());
    }

    #[test]
    fn dash() {
        let url = "https://www.youtube.com/watch?v=_KhZ7F-jOlI";
        assert_eq!("_KhZ7F-jOlI", extract_id(url).unwrap());
    }
}
