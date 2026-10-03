use anyhow::Result;
use lol_html::{HtmlRewriter, Settings, element};
use regex::{Captures, Regex};
use std::cell::RefCell;
use std::rc::Rc;

use crate::asset::AssetStore;

pub struct HtmlUnbundler {
    asset_store: Rc<RefCell<AssetStore>>,
    script_store: Rc<RefCell<AssetStore>>,
    css_url_re: Regex,
    script_tag_re: Regex,
    script_type_re: Regex,
    script_src_attr_re: Regex,
    min_script_bytes: u32,
}

impl HtmlUnbundler {
    #[must_use]
    pub fn new(
        asset_store: Rc<RefCell<AssetStore>>,
        script_store: Rc<RefCell<AssetStore>>,
        min_script_bytes: u32,
    ) -> Self {
        let css_url_re = Regex::new(r#"url\(\s*['"]?(data:[^'")]+)['"]?\s*\)"#)
            .expect("invalid css regex");
        let script_tag_re =
            Regex::new(r"(?is)<script(?P<attrs>[^>]*)>(?P<content>.*?)</script>")
                .expect("invalid script tag regex");
        let script_type_re = Regex::new(r#"(?i)\btype\s*=\s*['"]?([^'">\s;]+)"#)
            .expect("invalid script type regex");
        let script_src_attr_re =
            Regex::new(r"(?i)\bsrc\s*=").expect("invalid script src attr regex");
        Self {
            asset_store,
            script_store,
            css_url_re,
            script_tag_re,
            script_type_re,
            script_src_attr_re,
            min_script_bytes,
        }
    }

    pub fn unbundle(&self, html: &str) -> Result<String> {
        let css_rewritten = self.rewrite_css_urls(html);
        let scripts_rewritten = self.rewrite_inline_scripts(&css_rewritten);
        self.rewrite_html_attributes(&scripts_rewritten)
    }

    fn rewrite_css_urls(&self, text: &str) -> String {
        let store = Rc::clone(&self.asset_store);
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

    fn rewrite_inline_scripts(&self, html: &str) -> String {
        let script_store = Rc::clone(&self.script_store);
        let min_bytes = usize::try_from(self.min_script_bytes).unwrap_or(usize::MAX);

        self.script_tag_re
            .replace_all(html, |caps: &Captures| {
                let attrs = &caps["attrs"];
                let content = &caps["content"];

                if !self.is_js_script(attrs) {
                    return caps[0].to_string();
                }

                let trimmed = content.trim();
                if trimmed.is_empty() || content.len() < min_bytes {
                    return caps[0].to_string();
                }

                let rel_path = script_store
                    .borrow_mut()
                    .process_bytes(content.as_bytes(), "js");

                rel_path.map_or_else(
                    || caps[0].to_string(),
                    |path| format_script_tag(attrs, &path),
                )
            })
            .into_owned()
    }

    fn is_js_script(&self, attrs: &str) -> bool {
        if self.script_src_attr_re.is_match(attrs) {
            return false;
        }
        self.script_type_re.captures(attrs).is_none_or(|caps| {
            let mime = caps[1].trim().to_ascii_lowercase();
            matches!(
                mime.as_str(),
                "text/javascript"
                    | "application/javascript"
                    | "text/ecmascript"
                    | "application/ecmascript"
                    | "module"
            )
        })
    }

    fn rewrite_html_attributes(&self, html: &str) -> Result<String> {
        let asset_store = Rc::clone(&self.asset_store);
        let script_store = Rc::clone(&self.script_store);
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
                        .and_then(|s| {
                            if el.tag_name() == "script" {
                                script_store.borrow_mut().process_data_url(&s)
                            } else {
                                asset_store.borrow_mut().process_data_url(&s)
                            }
                        })
                    {
                        el.set_attribute("src", &src)?;
                    }

                    if let Some(href) = el
                        .get_attribute("href")
                        .filter(|s| s.trim_start().starts_with("data:"))
                        .and_then(|s| asset_store.borrow_mut().process_data_url(&s))
                    {
                        el.set_attribute("href", &href)?;
                    }

                    if let Some(poster) = el
                        .get_attribute("poster")
                        .filter(|s| s.trim_start().starts_with("data:"))
                        .and_then(|s| asset_store.borrow_mut().process_data_url(&s))
                    {
                        el.set_attribute("poster", &poster)?;
                    }

                    if let Some(srcset) =
                        el.get_attribute("srcset").filter(|s| s.contains("data:"))
                    {
                        let new_srcset =
                            rewrite_srcset(&srcset, &mut asset_store.borrow_mut());
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

fn format_script_tag(attrs: &str, src: &str) -> String {
    let trimmed_attrs = attrs.trim();
    if trimmed_attrs.is_empty() {
        format!(r#"<script src="{src}"></script>"#)
    } else {
        format!(r#"<script {trimmed_attrs} src="{src}"></script>"#)
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
