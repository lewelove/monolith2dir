use anyhow::Result;
use lol_html::{HtmlRewriter, Settings, element};
use regex::{Captures, Regex};
use std::cell::RefCell;
use std::rc::Rc;

use crate::asset::AssetStore;

pub struct HtmlUnbundler {
    store: Rc<RefCell<AssetStore>>,
    css_url_re: Regex,
}

impl HtmlUnbundler {
    #[must_use]
    pub fn new(store: Rc<RefCell<AssetStore>>) -> Self {
        let css_url_re = Regex::new(r#"url\(\s*['"]?(data:[^'")]+)['"]?\s*\)"#)
            .expect("invalid css regex");
        Self { store, css_url_re }
    }

    pub fn unbundle(&self, html: &str) -> Result<String> {
        let css_rewritten = self.rewrite_css_urls(html);
        self.rewrite_html_attributes(&css_rewritten)
    }

    fn rewrite_css_urls(&self, text: &str) -> String {
        let store = Rc::clone(&self.store);
        self.css_url_re
            .replace_all(text, |caps: &Captures| {
                let data_url = &caps[1];
                store.borrow_mut().process_data_url(data_url).map_or_else(
                    || caps[0].to_string(),
                    |rel_path| format!(r#"url("{rel_path}")"#),
                )
            })
            .into_owned()
    }

    fn rewrite_html_attributes(&self, html: &str) -> Result<String> {
        let store = Rc::clone(&self.store);
        let mut output = Vec::with_capacity(html.len());

        let settings = Settings::new()
            .append_element_content_handler(element!("base", |el| {
                el.remove();
                Ok(())
            }))
            .append_element_content_handler(element!(
                "*[src], *[href], *[poster], *[srcset]",
                move |el| {
                    if let Some(src) = el
                        .get_attribute("src")
                        .filter(|s| s.trim_start().starts_with("data:"))
                        .and_then(|s| store.borrow_mut().process_data_url(&s))
                    {
                        el.set_attribute("src", &src)?;
                    }

                    if let Some(href) = el
                        .get_attribute("href")
                        .filter(|s| s.trim_start().starts_with("data:"))
                        .and_then(|s| store.borrow_mut().process_data_url(&s))
                    {
                        el.set_attribute("href", &href)?;
                    }

                    if let Some(poster) = el
                        .get_attribute("poster")
                        .filter(|s| s.trim_start().starts_with("data:"))
                        .and_then(|s| store.borrow_mut().process_data_url(&s))
                    {
                        el.set_attribute("poster", &poster)?;
                    }

                    if let Some(srcset) =
                        el.get_attribute("srcset").filter(|s| s.contains("data:"))
                    {
                        let new_srcset = rewrite_srcset(&srcset, &mut store.borrow_mut());
                        el.set_attribute("srcset", &new_srcset)?;
                    }

                    Ok(())
                }
            ));

        let mut rewriter = HtmlRewriter::new(settings, |chunk: &[u8]| {
            output.extend_from_slice(chunk);
        });

        rewriter.write(html.as_bytes())?;
        rewriter.end()?;

        Ok(String::from_utf8_lossy(&output).into_owned())
    }
}

fn rewrite_srcset(srcset: &str, store: &mut AssetStore) -> String {
    let mut parts = Vec::new();
    for entry in srcset.split(", ") {
        let trimmed = entry.trim();
        if !trimmed.starts_with("data:") {
            parts.push(trimmed.to_string());
            continue;
        }

        if let Some((url_part, descriptor)) = trimmed.split_once(' ') {
            let entry_str = store.process_data_url(url_part).map_or_else(
                || trimmed.to_string(),
                |rel_path| format!("{rel_path} {descriptor}"),
            );
            parts.push(entry_str);
        } else {
            let entry_str = store
                .process_data_url(trimmed)
                .unwrap_or_else(|| trimmed.to_string());
            parts.push(entry_str);
        }
    }
    parts.join(", ")
}
