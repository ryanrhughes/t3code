//! Composer attachments: pasted images, dropped or picked files. Each uploads as soon as it's
//! added (`attachments.createUploadUrl`, then the bytes), so a send only waits on the slow ones.
use rune::{El, Tx, col, row, text};
use rune_kit::{IconButton, IconButtonVariant};
use serde_json::{Value, json};
use t3_client::attachments::{
    IMAGE_TYPES, MAX_IMAGE_BYTES, file_type, image_type, upload_blocking,
};

use crate::theme::Look;
use crate::{App, HANDLE};

/// `PROVIDER_SEND_TURN_MAX_FILE_BYTES`: no server takes a bigger file into a turn.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

pub struct Attachment {
    pub key: u64,
    /// The environment it was uploaded to; a send elsewhere can't use it.
    pub env_id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub image: bool,
    pub state: AttachState,
}

pub enum AttachState {
    Uploading,
    Ready(String),
    Failed(String),
}

impl Attachment {
    /// The message's reference to it, once uploaded.
    pub fn reference(&self) -> Option<Value> {
        let AttachState::Ready(id) = &self.state else {
            return None;
        };
        Some(json!({
            "type": if self.image { "image" } else { "file" },
            "id": id,
            "name": self.name,
            "mimeType": self.mime,
            "sizeBytes": self.size,
        }))
    }
}

impl App {
    /// The largest file this machine takes: what it advertises (`fileAttachments`), within the
    /// contract's limit (`clampFileAttachmentUploadBytes`).
    fn max_file_bytes(&self) -> u64 {
        self.config()
            .and_then(|c| {
                c.environment
                    .capabilities
                    .get("fileAttachments")?
                    .get("maxUploadBytes")?
                    .as_u64()
            })
            .map_or(MAX_FILE_BYTES, |n| n.min(MAX_FILE_BYTES))
    }

    /// Adds files from a drop or the picker; images go up as images, the rest as files.
    pub fn attach_paths(&mut self, paths: &[std::path::PathBuf], tx: &mut Tx) {
        for path in paths {
            let name = path
                .file_name()
                .map_or("file".into(), |n| n.to_string_lossy().into_owned());
            if path.is_dir() {
                self.toast(format!("{name} is a folder; attach files instead"), tx);
                continue;
            }
            match std::fs::metadata(path) {
                Ok(m) if m.len() > self.max_file_bytes() => {
                    self.toast(format!("{name} is too large to attach"), tx);
                    continue;
                }
                Err(e) => {
                    self.toast(format!("Couldn't read {name}: {e}"), tx);
                    continue;
                }
                _ => {}
            }
            match std::fs::read(path) {
                Ok(bytes) => self.attach_bytes(name, bytes, tx),
                Err(e) => self.toast(format!("Couldn't read {name}: {e}"), tx),
            }
        }
    }

    pub fn attach_bytes(&mut self, name: String, bytes: Vec<u8>, tx: &mut Tx) {
        let size = bytes.len() as u64;
        if size == 0 {
            self.toast(format!("{name} is empty"), tx);
            return;
        }
        let image = image_type(&bytes).filter(|_| size <= MAX_IMAGE_BYTES);
        let mime = image.unwrap_or_else(|| file_type(&name)).to_owned();
        let (Some(env), Some(handle)) = (self.env(), HANDLE.get().cloned()) else {
            self.toast("Connect to a server to attach files", tx);
            return;
        };
        let (Some(session), env_id, http_base) = (
            env.session.clone(),
            env.cred.environment_id.clone(),
            env.cred.http_base.clone(),
        ) else {
            return;
        };
        self.next_attachment += 1;
        let key = self.next_attachment;
        let is_image = image.is_some_and(|m| IMAGE_TYPES.contains(&m));
        self.attachments.push(Attachment {
            key,
            env_id,
            name: name.clone(),
            mime: mime.clone(),
            size,
            image: is_image,
            state: AttachState::Uploading,
        });
        let input = json!({
            "type": if is_image { "image" } else { "file" },
            "name": name,
            "mimeType": mime,
            "sizeBytes": size,
        });
        session.call("attachments.createUploadUrl", input, move |minted| {
            let finish = move |result: Result<String, String>| {
                handle.update(move |s: &mut App, _| {
                    if let Some(a) = s.attachments.iter_mut().find(|a| a.key == key) {
                        a.state = match result {
                            Ok(id) => AttachState::Ready(id),
                            Err(e) => AttachState::Failed(e),
                        };
                    }
                })
            };
            let minted = match minted {
                Ok(v) => v,
                Err(e) => return finish(Err(e)),
            };
            let (Some(id), Some(url)) = (
                minted["attachmentId"].as_str().map(str::to_owned),
                minted["relativeUrl"].as_str().map(str::to_owned),
            ) else {
                return finish(Err("The server didn't return an upload URL".into()));
            };
            std::thread::spawn(move || {
                finish(
                    upload_blocking(&http_base, &url, bytes)
                        .map(|()| id)
                        .map_err(|e| e.to_string()),
                )
            });
        });
    }

    pub fn detach(&mut self, key: u64) {
        self.attachments.retain(|a| a.key != key);
    }

    /// What a send attaches, or why it can't yet.
    pub fn ready_attachments(&self) -> Result<Vec<Value>, &'static str> {
        let env = self.env().map(|e| e.cred.environment_id.as_str());
        let mine = self
            .attachments
            .iter()
            .filter(|a| Some(a.env_id.as_str()) == env);
        let mut out = Vec::new();
        for a in mine {
            match &a.state {
                AttachState::Uploading => {
                    return Err("Wait for the attachments to finish uploading");
                }
                AttachState::Failed(_) => {
                    return Err("Remove the attachment that failed to upload");
                }
                AttachState::Ready(_) => out.extend(a.reference()),
            }
        }
        Ok(out)
    }

    /// Opens the system file picker (the XDG portal on Linux) and attaches what's chosen.
    pub fn pick_attachment(&mut self, tx: &mut Tx) {
        tx.open_file(
            rune::FileDialogOptions {
                title: "Attach a file".into(),
                ..Default::default()
            },
            |s: &mut App, result, tx| {
                if let Ok(Some(path)) = result {
                    s.attach_paths(&[path], tx);
                }
            },
        );
    }
}

fn size_label(n: u64) -> String {
    let kb = n.div_ceil(1024).max(1);
    if kb >= 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{kb} KB")
    }
}

/// The composer's attachment chips: name, size or state, and a remove button.
pub fn chips(s: &App, look: &Look) -> Option<El> {
    let env = s.env().map(|e| e.cred.environment_id.as_str());
    let shown: Vec<&Attachment> = s
        .attachments
        .iter()
        .filter(|a| Some(a.env_id.as_str()) == env)
        .collect();
    if shown.is_empty() {
        return None;
    }
    let k = &look.kit;
    let mut wrap = row().w_full().gap(6.0).flex_wrap().px(4.0);
    for a in shown {
        let (detail, color) = match &a.state {
            AttachState::Uploading => ("Uploading…".to_owned(), look.muted),
            AttachState::Ready(_) => (size_label(a.size), look.muted),
            AttachState::Failed(e) => (format!("Failed: {e}"), look.error_fg),
        };
        let key = a.key;
        let remove = IconButton::new(&rune_kit::glyphs::CLOSE)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(rune_kit::ControlSize::Small)
            .label(format!("Remove {}", a.name))
            .on_press(move |s: &mut App| s.detach(key))
            .build(k)
            .key("remove");
        let icon = if a.image {
            &rune_kit::glyphs::FILE_IMAGE
        } else {
            &rune_kit::glyphs::FILE
        };
        wrap = wrap.child(
            row()
                .key(format!("attachment-{key}"))
                .gap(6.0)
                .pl(8.0)
                .pr(2.0)
                .py(2.0)
                .center_y()
                .radius(k.radius.md)
                .fill(look.surface)
                .border(1.0, look.border)
                .max_w(280.0)
                .child(rune::icon(icon, 14.0).color(look.muted))
                .child(
                    col()
                        .shrink()
                        .min_w(0.0)
                        .child(
                            text(a.name.clone())
                                .font(k.ty.small)
                                .color(look.text)
                                .truncate(),
                        )
                        .child(text(detail).font(k.ty.small).color(color).truncate()),
                )
                .child(remove),
        );
    }
    Some(wrap)
}
