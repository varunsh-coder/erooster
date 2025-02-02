use color_eyre::eyre::bail;
use futures::{Sink, SinkExt};
use tracing::instrument;

use crate::commands::{CommandData, Data};

pub struct Ehlo<'a> {
    pub data: &'a Data,
}

impl Ehlo<'_> {
    #[instrument(skip(self, hostname, lines, command_data))]
    pub async fn exec<S, E>(
        &self,
        hostname: &str,
        lines: &mut S,
        command_data: &CommandData<'_>,
    ) -> color_eyre::eyre::Result<()>
    where
        E: std::error::Error + std::marker::Sync + std::marker::Send + 'static,
        S: Sink<String, Error = E> + std::marker::Unpin + std::marker::Send,
    {
        if command_data.arguments.is_empty() {
            bail!("Invalid EHLO arguments: {:?}", command_data.arguments);
        }
        let mut write_lock = self.data.con_state.write().await;
        write_lock.ehlo = Some(command_data.arguments[0].to_string());
        lines.feed(format!("250-{hostname}")).await?;
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
