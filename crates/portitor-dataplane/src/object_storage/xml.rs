// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The few XML pieces the object storage client reads and writes: the
//! text of one element of an answer (`UploadId`, an error `Code`), and the
//! body of `CompleteMultipartUpload`. No XML crate: the answers are small
//! documents of the S3 API, read for one plain-text element each, and
//! the only document written has two fields per part.

use std::fmt::Write as _;

/// The text of the first element `<name>...</name>` in `document`, with
/// the five predefined XML entities turned back into characters. `None`
/// when there is no such element. Attributes, namespaces and nested
/// elements are not looked at: none of the elements read has them.
#[must_use]
pub fn text_of(document: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = document.find(&open)? + open.len();
    let length = document.get(start..)?.find(&close)?;
    let raw = document.get(start..start + length)?;
    Some(unescape(raw))
}

/// `&lt;` `&gt;` `&quot;` `&apos;` and `&amp;` back to characters
/// (`&amp;` last, so `&amp;lt;` stays `&lt;`).
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `text` with `& < > " '` written as entities.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The body of `CompleteMultipartUpload` (S3 API reference): each part's
/// number and the `ETag` its upload answered with, in order.
#[must_use]
pub fn complete_multipart_upload(parts: &[(u32, String)]) -> String {
    let mut document = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        // writing to a String cannot fail
        let _ = write!(
            document,
            "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>",
            escape(etag)
        );
    }
    document.push_str("</CompleteMultipartUpload>");
    document
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The answer of the example "Initiate a multipart upload" of the S3
    /// API reference (`CreateMultipartUpload`), shortened.
    #[test]
    fn the_upload_id_is_read_from_the_answer() {
        let answer = r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Bucket>example-bucket</Bucket>
  <Key>example-object</Key>
  <UploadId>VXBsb2FkIElEIGZvciA2aWWpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA</UploadId>
</InitiateMultipartUploadResult>"#;
        assert_eq!(
            text_of(answer, "UploadId").as_deref(),
            Some("VXBsb2FkIElEIGZvciA2aWWpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA")
        );
        assert_eq!(text_of(answer, "ETag"), None);
        assert_eq!(
            text_of("<Code>a&amp;lt;b</Code>", "Code").as_deref(),
            Some("a&lt;b")
        );
        assert_eq!(text_of("<Code>open", "Code"), None);
    }

    #[test]
    fn the_completion_lists_every_part_with_its_etag() {
        let parts = [(1, "\"a1\"".to_owned()), (2, "\"b2\"".to_owned())];
        assert_eq!(
            complete_multipart_upload(&parts),
            "<CompleteMultipartUpload>\
             <Part><PartNumber>1</PartNumber><ETag>&quot;a1&quot;</ETag></Part>\
             <Part><PartNumber>2</PartNumber><ETag>&quot;b2&quot;</ETag></Part>\
             </CompleteMultipartUpload>"
        );
    }
}
