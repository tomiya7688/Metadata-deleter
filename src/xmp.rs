//! Emit minimal, safe XMP for explicitly replaced fields. Original XML is
//! discarded, so unknown elements cannot carry old identity information through.
use crate::config::Config;

fn escaped(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn scalar(xml: &mut String, field: &str, value: &str) {
    xml.push('<'); xml.push_str(field); xml.push('>');
    xml.push_str(&escaped(value));
    xml.push_str("</"); xml.push_str(field); xml.push('>');
}

fn alt(xml: &mut String, field: &str, value: &str) {
    xml.push('<'); xml.push_str(field); xml.push_str("><rdf:Alt><rdf:li xml:lang=\"x-default\">");
    xml.push_str(&escaped(value));
    xml.push_str("</rdf:li></rdf:Alt></"); xml.push_str(field); xml.push('>');
}

pub fn replacement(cfg: &Config) -> Option<Vec<u8>> {
    let replacements = [
        &cfg.creator, &cfg.copyright, &cfg.software, &cfg.comments,
        &cfg.title, &cfg.description, &cfg.keywords,
        &cfg.camera_make, &cfg.camera_model, &cfg.lens,
        &cfg.device_serial, &cfg.image_id,
    ];
    if !replacements.iter().any(|x| x.replacement().is_some()) { return None; }
    let mut xml = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:tiff=\"http://ns.adobe.com/tiff/1.0/\" xmlns:aux=\"http://ns.adobe.com/exif/1.0/aux/\" xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\">",
    );
    if let Some(v) = cfg.creator.replacement() {
        xml.push_str("<dc:creator><rdf:Seq><rdf:li>");
        xml.push_str(&escaped(v));
        xml.push_str("</rdf:li></rdf:Seq></dc:creator>");
    }
    if let Some(v) = cfg.copyright.replacement() { alt(&mut xml, "dc:rights", v); }
    if let Some(v) = cfg.software.replacement() { scalar(&mut xml, "xmp:CreatorTool", v); }
    if let Some(v) = cfg.title.replacement() { alt(&mut xml, "dc:title", v); }
    if let Some(v) = cfg.description.replacement() { alt(&mut xml, "dc:description", v); }
    if let Some(v) = cfg.comments.replacement() {
        if cfg.description.replacement().is_none() { alt(&mut xml, "dc:description", v); }
        else { scalar(&mut xml, "xmp:Label", v); }
    }
    if let Some(v) = cfg.keywords.replacement() {
        xml.push_str("<dc:subject><rdf:Bag><rdf:li>");
        xml.push_str(&escaped(v));
        xml.push_str("</rdf:li></rdf:Bag></dc:subject>");
    }
    if let Some(v) = cfg.camera_make.replacement() { scalar(&mut xml, "tiff:Make", v); }
    if let Some(v) = cfg.camera_model.replacement() { scalar(&mut xml, "tiff:Model", v); }
    if let Some(v) = cfg.lens.replacement() { scalar(&mut xml, "aux:Lens", v); }
    if let Some(v) = cfg.device_serial.replacement() { scalar(&mut xml, "aux:SerialNumber", v); }
    if let Some(v) = cfg.image_id.replacement() { scalar(&mut xml, "xmpMM:DocumentID", v); }
    xml.push_str("</rdf:Description></rdf:RDF></x:xmpmeta>");
    Some(xml.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_values_and_emits_standard_fields() {
        let cfg = Config::parse("[keep]\ncreator=\"tomiya & <someone>\"\ntitle=\"New title\"\ncamera_make=\"Acme\"\nkeywords=\"holiday\"\n").unwrap();
        let xml = String::from_utf8(replacement(&cfg).unwrap()).unwrap();
        assert!(xml.contains("tomiya &amp; &lt;someone&gt;"));
        assert!(!xml.contains("<someone>"));
        assert!(xml.contains("<dc:title>"));
        assert!(xml.contains("<tiff:Make>Acme</tiff:Make>"));
        assert!(xml.contains("<dc:subject>"));
        assert!(replacement(&Config::default()).is_none());
    }
}
