/// Unicode-aware slugify (transliterates non-ASCII where possible, then
/// lowercases and hyphenates).
pub fn slugify(title: &str) -> String {
    let base = slug::slugify(title);
    if base.is_empty() {
        "post".to_string()
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_transliterates_and_hyphenates() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Café con leche"), "cafe-con-leche");
        assert_eq!(slugify(""), "post");
    }
}
