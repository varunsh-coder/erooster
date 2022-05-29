use crate::{
    backend::storage::{MailEntry, MailEntryType, MailStorage, Storage},
    imap_commands::{parsers::fetch_arguments, CommandData, Data},
    imap_servers::state::State,
};
use futures::{channel::mpsc::SendError, stream, Sink, SinkExt, StreamExt};
use std::sync::Arc;
use tracing::debug;

pub struct Uid<'a> {
    pub data: &'a Data,
}

impl Uid<'_> {
    pub async fn exec<S>(
        &self,
        lines: &mut S,
        command_data: &CommandData<'_>,
        storage: Arc<Storage>,
    ) -> color_eyre::eyre::Result<()>
    where
        S: Sink<String, Error = SendError> + std::marker::Unpin + std::marker::Send,
    {
        debug!("Arguments: {:?}", command_data.arguments);
        if command_data.arguments[0].to_lowercase() == "fetch" {
            // TODO handle the various request types defined in https://www.rfc-editor.org/rfc/rfc9051.html#name-fetch-command
            // TODO handle * as "everything"
            // TODO make this code also available to the pure FETCH command
            if let State::Selected(folder, _) = &self.data.con_state.read().await.state {
                let mails: Vec<MailEntryType> = storage.list_all(folder.to_string());

                let range = command_data.arguments[1].split(':').collect::<Vec<_>>();
                let start = range[0].parse::<i64>().unwrap_or(1);
                let end = range[1];
                let end_int = end.parse::<i64>().unwrap_or(i64::max_value());
                let filtered_mails: Vec<MailEntryType> = if end == "*" {
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
                };

                let fetch_args = command_data.arguments[2..].to_vec().join(" ");
                let (_, args) =
                    fetch_arguments(fetch_args).expect("Failed to parse fetch arguments");
                debug!("Fetch args: {:?}", args);
                for mail in filtered_mails {
                    let uid = mail.uid().await?;

                    let line = format!("* {} FETCH ", uid);
                }

                lines
                    .feed(format!("{} Ok UID FETCH completed", command_data.tag))
                    .await?;
                lines.flush().await?;
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
