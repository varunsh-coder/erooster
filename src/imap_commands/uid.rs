use crate::{
    backend::storage::{MailEntry, MailEntryType, MailStorage, Storage},
    config::Config,
    imap_commands::{
        parsers::{fetch_arguments, FetchArguments, FetchAttributes},
        CommandData, Data,
    },
    imap_servers::state::State,
};
use futures::{channel::mpsc::SendError, stream, Sink, SinkExt, StreamExt};
use std::{path::Path, sync::Arc};
use tracing::{debug, error};

pub struct Uid<'a> {
    pub data: &'a Data,
}

impl Uid<'_> {
    #[allow(clippy::too_many_lines)]
    pub async fn exec<S>(
        &self,
        lines: &mut S,
        config: Arc<Config>,
        command_data: &CommandData<'_>,
        storage: Arc<Storage>,
    ) -> color_eyre::eyre::Result<()>
    where
        S: Sink<String, Error = SendError> + std::marker::Unpin + std::marker::Send,
    {
        if command_data.arguments[0].to_lowercase() == "fetch" {
            // TODO handle the various request types defined in https://www.rfc-editor.org/rfc/rfc9051.html#name-fetch-command
            // TODO handle * as "everything"
            // TODO make this code also available to the pure FETCH command
            if let State::Selected(folder, _) = &self.data.con_state.read().await.state {
                let mut folder = folder.replace('/', ".");
                folder.insert(0, '.');
                folder.remove_matches('"');
                folder = folder.replace(".INBOX", "INBOX");
                let mailbox_path = Path::new(&config.mail.maildir_folders)
                    .join(self.data.con_state.read().await.username.clone().unwrap())
                    .join(folder.clone());
                let mails: Vec<MailEntryType> = storage.list_all(
                    mailbox_path
                        .into_os_string()
                        .into_string()
                        .expect("Failed to convert path. Your system may be incompatible"),
                );

                let filtered_mails: Vec<MailEntryType> = if command_data.arguments[1].contains(':')
                {
                    let range = command_data.arguments[1].split(':').collect::<Vec<_>>();
                    let start = range[0].parse::<i64>().unwrap_or(1);
                    let end = range[1];
                    let end_int = end.parse::<i64>().unwrap_or(i64::max_value());
                    if end == "*" {
                        stream::iter(mails)
                            .filter_map(|mail: MailEntryType| async {
                                if let Ok(id) = mail.uid().await {
                                    (id >= start).then(|| mail)
                                } else {
                                    None
                                }
                            })
                            .collect()
                            .await
                    } else {
                        stream::iter(mails)
                            .filter_map(|mail: MailEntryType| async move {
                                if let Ok(id) = mail.uid().await {
                                    (id >= start && id <= end_int).then(|| mail)
                                } else {
                                    None
                                }
                            })
                            .collect()
                            .await
                    }
                } else {
                    let wanted_id = command_data.arguments[1].parse::<i64>().unwrap_or(1);
                    stream::iter(mails)
                        .filter_map(|mail: MailEntryType| async {
                            if let Ok(id) = mail.uid().await {
                                (id == wanted_id).then(|| mail)
                            } else {
                                None
                            }
                        })
                        .collect()
                        .await
                };

                let fetch_args = command_data.arguments[2..].to_vec().join(" ");
                let fetch_args_str = &fetch_args[1..fetch_args.len() - 1];
                debug!("Fetch args: {}", fetch_args_str);
                match fetch_arguments(fetch_args_str) {
                    Ok((_, args)) => {
                        for mut mail in filtered_mails {
                            let uid = mail.uid().await?;
                            if let Some(resp) = generate_response(args.clone(), &mut mail) {
                                lines
                                    .feed(format!("* {} FETCH (UID {} {})", uid, uid, resp))
                                    .await?;
                            }
                        }

                        lines
                            .feed(format!("{} Ok UID FETCH completed", command_data.tag))
                            .await?;
                        lines.flush().await?;
                    }
                    Err(e) => {
                        error!("Failed to parse fetch arguments: {}", e);
                        lines
                            .send(format!("{} BAD Unable to parse", command_data.tag))
                            .await?;
                    }
                }
            } else {
                lines
                    .feed(format!(
                        "{} NO [TRYCREATE] No mailbox selected",
                        command_data.tag
                    ))
                    .await?;
                lines.flush().await?;
            }
        } else if command_data.arguments[0].to_lowercase() == "copy" {
            // TODO implement other commands
            lines
                .send(format!("{} BAD Not supported", command_data.tag))
                .await?;
        } else if command_data.arguments[0].to_lowercase() == "move" {
            lines
                .send(format!("{} BAD Not supported", command_data.tag))
                .await?;
        } else if command_data.arguments[0].to_lowercase() == "expunge" {
            lines
                .send(format!("{} BAD Not supported", command_data.tag))
                .await?;
        } else if command_data.arguments[0].to_lowercase() == "search" {
            lines
                .send(format!("{} BAD Not supported", command_data.tag))
                .await?;
        }
        Ok(())
    }
}

fn generate_response(arg: FetchArguments, mail: &mut MailEntryType) -> Option<String> {
    match arg {
        FetchArguments::All => None,
        FetchArguments::Fast => None,
        FetchArguments::Full => None,
        FetchArguments::Single(single_arg) => generate_response_for_attributes(single_arg, mail),
        FetchArguments::List(args) => {
            let mut resp = String::new();
            for arg in args {
                if let Some(extra_resp) = generate_response_for_attributes(arg, mail) {
                    if resp.is_empty() {
                        resp = extra_resp;
                    } else {
                        resp.push_str(&format!(" {}", extra_resp));
                    }
                }
            }
            Some(resp)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn generate_response_for_attributes(
    attr: FetchAttributes,
    mail: &mut MailEntryType,
) -> Option<String> {
    match attr {
        FetchAttributes::Envelope => None,
        FetchAttributes::RFC822Header => {
            if let Ok(headers_vec) = mail.headers() {
                let headers = headers_vec
                    .iter()
                    .map(|header| format!("{}: {}", header.get_key(), header.get_value()))
                    .collect::<Vec<_>>()
                    .join("\r\n");

                Some(format!("RFC822.HEADER{}", headers))
            } else {
                Some(String::from("RFC822.HEADER\r\n"))
            }
        }
        FetchAttributes::Flags => {
            let mut flags = String::new();
            if mail.is_draft() {
                flags = format!("{} \\Draft", flags);
            }
            if mail.is_flagged() {
                flags = format!("{} \\Flagged", flags);
            }
            if mail.is_seen() {
                flags = format!("{} \\Seen", flags);
            }
            if mail.is_replied() {
                flags = format!("{} \\Answered", flags);
            }
            if mail.is_trashed() {
                flags = format!("{} \\Deleted", flags);
            }

            Some(format!("FLAGS ({})", flags))
        }
        FetchAttributes::InternalDate => None,
        FetchAttributes::RFC822Size => {
            if let Ok(parsed) = mail.parsed() {
                let size = match parsed.get_body_encoded() {
                    mailparse::body::Body::Base64(b) => b.get_raw().len(),
                    mailparse::body::Body::QuotedPrintable(q) => q.get_raw().len(),
                    mailparse::body::Body::SevenBit(s) => s.get_raw().len(),
                    mailparse::body::Body::EightBit(e) => e.get_raw().len(),
                    mailparse::body::Body::Binary(b) => b.get_raw().len(),
                };
                Some(format!("RFC822.SIZE {}", size))
            } else {
                Some(String::from("RFC822.SIZE 0"))
            }
        }
        FetchAttributes::Uid => None,
        FetchAttributes::BodyStructure => None,
        FetchAttributes::BodySection(_, _) => None,
        FetchAttributes::BodyPeek(section_text, _) => {
            if let Some(section_text) = section_text {
                match section_text {
                    super::parsers::SectionText::Header => {
                        Some(String::from("BODY.PEEK[HEADER.FIELDS]\r\n"))
                    }
                    super::parsers::SectionText::Text => None,
                    super::parsers::SectionText::HeaderFields(headers_requested_vec) => {
                        if let Ok(headers_vec) = mail.headers() {
                            let lower_headers_requested_vec: Vec<_> = headers_requested_vec
                                .iter()
                                .map(|header| header.to_lowercase())
                                .collect();
                            let headers = headers_vec
                                .iter()
                                .filter(|header| {
                                    lower_headers_requested_vec
                                        .contains(&header.get_key().to_lowercase())
                                })
                                .map(|header| {
                                    format!("{}: {}", header.get_key(), header.get_value())
                                })
                                .collect::<Vec<_>>()
                                .join("\r\n");

                            Some(format!(
                                "BODY.PEEK[HEADER.FIELDS] {{{}}}\r\n{}",
                                headers.as_bytes().len(),
                                headers
                            ))
                        } else {
                            Some(String::from("BODY.PEEK[HEADER.FIELDS]\r\n"))
                        }
                    }
                    super::parsers::SectionText::HeaderFieldsNot(headers_requested_vec) => {
                        if let Ok(headers_vec) = mail.headers() {
                            let lower_headers_requested_vec: Vec<_> = headers_requested_vec
                                .iter()
                                .map(|header| header.to_lowercase())
                                .collect();

                            let headers = headers_vec
                                .iter()
                                .filter(|header| {
                                    !lower_headers_requested_vec
                                        .contains(&header.get_key().to_lowercase())
                                })
                                .map(|header| {
                                    format!("{}: {}", header.get_key(), header.get_value())
                                })
                                .collect::<Vec<_>>()
                                .join("\r\n");
                            Some(format!(
                                "BODY.PEEK[HEADER.FIELDS] {{{}}}\r\n{}",
                                headers.as_bytes().len(),
                                headers
                            ))
                        } else {
                            Some(String::from("BODY.PEEK[HEADER.FIELDS]\r\n"))
                        }
                    }
                }
            } else {
                Some(String::from("BODY.PEEK[HEADER.FIELDS]\r\n"))
            }
        }
        FetchAttributes::Binary(_, _) => None,
        FetchAttributes::BinaryPeek(_, _) => None,
        FetchAttributes::BinarySize(_) => None,
    }
}
