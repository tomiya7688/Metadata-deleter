//! Build a minimal standards-based XMP packet containing ONLY requested text
//! replacements. Original XMP is never copied into the new packet.
use crate::config::Config;

fn escaped(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        match ch {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(ch),
        }
    }
    result
}

pub fn replacement(config: &Config) -> Option<Vec<u8>> {
    let creator = config.creator.replacement();
    let copyright = config.copyright.replacement();
    let software = config.software.replacement();
    let comments = config.comments.replacement();
    if creator.is_none() && copyright.is_none() && software.is_none() && comments.is_none() {
        return None;
    }
    let mut xml = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">",
    );
    if let Some(s) = creator {
        xml.push_str("<dc:creator><rdf:Seq><rdf:li>");
        xml.push_str(&escaped(s));
        xml.push_str("</rdf:li></rdf:Seq></dc:creator>");
    }
    if let Some(s) = copyright {
        xml.push_str("<dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">");
        xml.push_str(&escaped(s));
        xml.push_str("</rdf:li></rdf:Alt></dc:rights>");
    }
    if let Some(s) = software {
        xml.push_str("<xmp:CreatorTool>");
        xml.push_str(&escaped(s));
        xml.push_str("</xmp:CreatorTool>");
    }
    if let Some(s) = comments {
        xml.push_str("<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">");
        xml.push_str(&escaped(s));
        xml.push_str("</rdf:li></rdf:Alt></dc:description>");
    }
    xml.push_str("</rdf:Description></rdf:RDF></x:xmpmeta>");
    Some(xml.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_has_only_escaped_replacements() {
        let cfg = Config::parse("[keep]\ncreator = \"tomiya & <someone>\"\n").unwrap();
        let xml = String::from_utf8(replacement(&cfg).unwrap()).unwrap();
        assert!(xml.contains("tomiya &amp; &lt;someone&gt;"));
        assert!(!xml.contains("<someone>"));
        assert!(xml.contains("<dc:creator>"));
        assert!(!xml.contains("CreatorTool"));
        assert!(replacement(&Config::default()).is_none());
    }
}
