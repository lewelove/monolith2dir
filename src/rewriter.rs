use anyhow::Result;
use lol_html::html_content::{ContentType, Element};
use lol_html::{HtmlRewriter, Settings, element, end_tag, text};
use regex::{Captures, Regex};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::rc::Rc;

use crate::asset::AssetStore;

pub struct HtmlUnbundler {
    asset_store: Rc<RefCell<AssetStore>>,
    script_store: Rc<RefCell<AssetStore>>,
    style_store: Rc<RefCell<AssetStore>>,
    css_url_re: Regex,
    void_end_tag_re: Regex,
}

struct Pass1Data {
    scripts: VecDeque<Option<String>>,
    styles: VecDeque<Option<(String, Option<String>)>>,
}

impl HtmlUnbundler {
    #[must_use]
    pub fn new(
        asset_store: Rc<RefCell<AssetStore>>,
        script_store: Rc<RefCell<AssetStore>>,
        style_store: Rc<RefCell<AssetStore>>,
    ) -> Self {
        let css_url_re = Regex::new(r#"url\(\s*['"]?(data:[^'")]+)['"]?\s*\)"#)
            .expect("invalid css regex");
        let void_end_tag_re = Regex::new(r"(?i)</(meta|base|link|br|hr|img|input)>")
            .expect("invalid void end tag regex");
        Self {
            asset_store,
            script_store,
            style_store,
            css_url_re,
            void_end_tag_re,
        }
    }

    pub fn unbundle(&self, html: &str) -> Result<String> {
        let pass1 = self.extract_inline_content(html)?;
        self.rewrite_document(html, pass1)
    }

    fn extract_inline_content(&self, html: &str) -> Result<Pass1Data> {
        let scripts = Rc::new(RefCell::new(VecDeque::new()));
        let styles = Rc::new(RefCell::new(VecDeque::new()));

        let current_script = Rc::new(RefCell::new(None));
        let current_style = Rc::new(RefCell::new(None));

        let cur_script_el = Rc::clone(&current_script);
        let scripts_store_el = Rc::clone(&self.script_store);
        let scripts_out = Rc::clone(&scripts);

        let cur_style_el = Rc::clone(&current_style);
        let style_store_el = Rc::clone(&self.style_store);
        let styles_out = Rc::clone(&styles);
        let asset_store_style = Rc::clone(&self.asset_store);
        let css_re_style = self.css_url_re.clone();

        let cur_script_txt = Rc::clone(&current_script);
        let cur_style_txt = Rc::clone(&current_style);

        let settings = Settings::new()
            .append_element_content_handler(element!("script", move |el| {
                let is_exec = !el.has_attribute("src") && is_executable_script(el);
                if is_exec {
                    *cur_script_el.borrow_mut() = Some(String::new());
                    let cur = Rc::clone(&cur_script_el);
                    let store = Rc::clone(&scripts_store_el);
                    let out = Rc::clone(&scripts_out);
                    el.on_end_tag(end_tag!(move |_| {
                        let path = cur.borrow_mut().take().and_then(|code| {
                            if code.trim().is_empty() {
                                None
                            } else {
                                store.borrow_mut().process_bytes(code.as_bytes(), "js")
                            }
                        });
                        out.borrow_mut().push_back(path);
                        Ok(())
                    }))?;
                } else {
                    *cur_script_el.borrow_mut() = None;
                    scripts_out.borrow_mut().push_back(None);
                }
                Ok(())
            }))
            .append_element_content_handler(text!("script", move |chunk| {
                if let Some(buf) = cur_script_txt.borrow_mut().as_mut() {
                    buf.push_str(chunk.as_str());
                }
                Ok(())
            }))
            .append_element_content_handler(element!("style", move |el| {
                *cur_style_el.borrow_mut() = Some(String::new());
                let cur = Rc::clone(&cur_style_el);
                let media = el.get_attribute("media");
                let s_store = Rc::clone(&style_store_el);
                let a_store = Rc::clone(&asset_store_style);
                let out = Rc::clone(&styles_out);
                let re = css_re_style.clone();
                el.on_end_tag(end_tag!(move |_| {
                    let path = cur.borrow_mut().take().and_then(|css| {
                        if css.trim().is_empty() {
                            None
                        } else {
                            let rewritten =
                                rewrite_css_urls_internal(&css, &re, &a_store, "../");
                            s_store
                                .borrow_mut()
                                .process_bytes(rewritten.as_bytes(), "css")
                        }
                    });
                    out.borrow_mut().push_back(path.map(|p| (p, media.clone())));
                    Ok(())
                }))?;
                Ok(())
            }))
            .append_element_content_handler(text!("style", move |chunk| {
                if let Some(buf) = cur_style_txt.borrow_mut().as_mut() {
                    buf.push_str(chunk.as_str());
                }
                Ok(())
            }));

        let mut rewriter = HtmlRewriter::new(settings, |_chunk: &[u8]| {});
        rewriter.write(html.as_bytes())?;
        rewriter.end()?;

        let scripts = Rc::try_unwrap(scripts)
            .map_err(|_| anyhow::anyhow!("script queue unwrap failed"))?
            .into_inner();
        let styles = Rc::try_unwrap(styles)
            .map_err(|_| anyhow::anyhow!("style queue unwrap failed"))?
            .into_inner();

        Ok(Pass1Data { scripts, styles })
    }

    fn rewrite_document(&self, html: &str, pass1: Pass1Data) -> Result<String> {
        let mut output = Vec::with_capacity(html.len());
        let asset_store = Rc::clone(&self.asset_store);
        let script_store = Rc::clone(&self.script_store);
        let style_store = Rc::clone(&self.style_store);
        let css_re = self.css_url_re.clone();

        let script_items = Rc::new(RefCell::new(pass1.scripts));
        let style_items = Rc::new(RefCell::new(pass1.styles));

        let script_items_el = Rc::clone(&script_items);
        let style_items_el = Rc::clone(&style_items);

        let asset_store_attrs = Rc::clone(&asset_store);
        let script_store_attrs = Rc::clone(&script_store);
        let style_store_attrs = Rc::clone(&style_store);

        let settings = Settings::new()
            .append_element_content_handler(element!("base", |el| {
                el.remove();
                Ok(())
            }))
            .append_element_content_handler(element!("meta", |el| {
                if el
                    .get_attribute("http-equiv")
                    .is_some_and(|h| h.eq_ignore_ascii_case("content-security-policy"))
                {
                    el.remove();
                }
                Ok(())
            }))
            .append_element_content_handler(element!("script", move |el| {
                if let Some(Some(rel_path)) = script_items_el.borrow_mut().pop_front() {
                    el.set_inner_content("", ContentType::Html);
                    el.set_attribute("src", &rel_path)?;
                }
                Ok(())
            }))
            .append_element_content_handler(element!("style", move |el| {
                if let Some(Some((rel_path, media))) =
                    style_items_el.borrow_mut().pop_front()
                {
                    let link_tag = media.map_or_else(
                        || format!(r#"<link rel="stylesheet" href="{rel_path}">"#),
                        |m| {
                            format!(
                                r#"<link rel="stylesheet" media="{m}" href="{rel_path}">"#
                            )
                        },
                    );
                    el.before(&link_tag, ContentType::Html);
                    el.remove();
                }
                Ok(())
            }))
            .append_element_content_handler(element!(
                "*[src], *[href], *[poster], *[srcset], *[style]",
                move |el| {
                    rewrite_attributes(
                        el,
                        &asset_store_attrs,
                        &script_store_attrs,
                        &style_store_attrs,
                        &css_re,
                    )
                }
            ));

        let mut rewriter = HtmlRewriter::new(settings, |chunk: &[u8]| {
            output.extend_from_slice(chunk);
        });

        rewriter.write(html.as_bytes())?;
        rewriter.end()?;

        let unbundled = String::from_utf8_lossy(&output);
        Ok(self
            .void_end_tag_re
            .replace_all(&unbundled, "")
            .into_owned())
    }
}

fn rewrite_attributes(
    el: &mut Element,
    asset_store: &Rc<RefCell<AssetStore>>,
    script_store: &Rc<RefCell<AssetStore>>,
    style_store: &Rc<RefCell<AssetStore>>,
    css_re: &Regex,
) -> std::result::Result<(), Box<dyn Error + Send + Sync>> {
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
        .and_then(|s| {
            if is_stylesheet_link(el) {
                process_stylesheet_data_url(&s, css_re, asset_store, style_store)
            } else {
                asset_store.borrow_mut().process_data_url(&s)
            }
        })
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

    if let Some(srcset) = el.get_attribute("srcset").filter(|s| s.contains("data:")) {
        let new_srcset = rewrite_srcset(&srcset, &mut asset_store.borrow_mut());
        el.set_attribute("srcset", &new_srcset)?;
    }

    if let Some(style) = el.get_attribute("style").filter(|s| s.contains("data:")) {
        let rewritten = rewrite_css_urls_internal(&style, css_re, asset_store, "");
        el.set_attribute("style", &rewritten)?;
    }

    Ok(())
}

fn is_executable_script(el: &Element) -> bool {
    el.get_attribute("type").is_none_or(|t| {
        let mime = t.trim().to_ascii_lowercase();
        let base_mime = mime.split(';').next().unwrap_or("").trim();
        matches!(
            base_mime,
            "" | "text/javascript"
                | "application/javascript"
                | "text/ecmascript"
                | "application/ecmascript"
                | "module"
        )
    })
}

fn is_stylesheet_link(el: &Element) -> bool {
    el.tag_name() == "link"
        && el
            .get_attribute("rel")
            .is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet"))
}

fn rewrite_css_urls_internal(
    css: &str,
    re: &Regex,
    store: &Rc<RefCell<AssetStore>>,
    prefix: &str,
) -> String {
    let s = Rc::clone(store);
    re.replace_all(css, |caps: &Captures| {
        let data_url = &caps[1];
        s.borrow_mut().process_data_url(data_url).map_or_else(
            || caps[0].to_string(),
            |rel_path| format!(r#"url("{prefix}{rel_path}")"#),
        )
    })
    .into_owned()
}

fn process_stylesheet_data_url(
    raw_url: &str,
    css_re: &Regex,
    asset_store: &Rc<RefCell<AssetStore>>,
    style_store: &Rc<RefCell<AssetStore>>,
) -> Option<String> {
    let trimmed = raw_url.trim();
    let data_url = data_url::DataUrl::process(trimmed).ok()?;
    let (body, _) = data_url.decode_to_vec().ok()?;
    let css_text = String::from_utf8_lossy(&body);
    let rewritten = rewrite_css_urls_internal(&css_text, css_re, asset_store, "../");
    style_store
        .borrow_mut()
        .process_bytes(rewritten.as_bytes(), "css")
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
