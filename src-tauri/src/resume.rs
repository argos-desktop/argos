/// Extract locally so the user can review the text before starting a session.
pub fn pdf_text(bytes: &[u8]) -> Result<String, &'static str> {
    let unreadable = "Could not read this PDF. Paste the résumé text instead.";
    let document = lopdf::Document::load_mem(bytes).map_err(|_| unreadable)?;
    if document.is_encrypted() {
        return Err("Unlock the PDF or paste the résumé text.");
    }
    let pages: Vec<_> = document.get_pages().into_keys().collect();
    let text = document.extract_text(&pages).map_err(|_| unreadable)?;
    let text = text.trim();
    if text.is_empty() {
        return Err("This PDF has no selectable text. Paste the résumé text instead.");
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Stream, dictionary};

    fn one_page(content: &[u8]) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let stream = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => stream,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        doc.objects.insert(
            pages,
            dictionary! {
                "Type" => "Pages", "Count" => 1, "Kids" => vec![page.into()],
            }
            .into(),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn extracts_selectable_text_and_rejects_empty_or_invalid_pdfs() {
        let pdf = one_page(b"BT /F1 12 Tf (Example engineer) Tj ET");
        assert_eq!(pdf_text(&pdf).unwrap(), "Example engineer");
        assert!(pdf_text(&one_page(b"")).is_err());
        assert!(pdf_text(b"invalid").is_err());
    }
}
