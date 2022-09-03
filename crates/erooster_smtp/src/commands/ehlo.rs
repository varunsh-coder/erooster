use futures::{channel::mpsc::SendError, Sink, SinkExt};
use tracing::instrument;

use crate::commands::{CommandData, Data};

pub struct Ehlo<'a> {
    pub data: &'a Data,
}

impl Ehlo<'_> {
    #[instrument(skip(self, hostname, lines, command_data))]
    pub async fn exec<S>(
        &self,
        hostname: String,
        lines: &mut S,
        command_data: &CommandData<'_>,
    ) -> color_eyre::eyre::Result<()>
    where
        S: Sink<String, Error = SendError> + std::marker::Unpin + std::marker::Send,
    {
        let mut write_lock = self.data.con_state.write().await;
        write_lock.ehlo = Some(command_data.arguments[0].to_string());
        lines.feed(format!("250-{}", hostname)).await?;
        lines.feed(String::from("250-ENHANCEDSTATUSCODES")).await?;
        if !write_lock.secure {
            lines.feed(String::from("250-STARTTLS")).await?;
        }
        if write_lock.secure {
            lines.feed(String::from("250-AUTH LOGIN PLAIN")).await?;
        }
        lines.feed(String::from("250 SMTPUTF8")).await?;
        lines.flush().await?;
        Ok(())
    }
}
