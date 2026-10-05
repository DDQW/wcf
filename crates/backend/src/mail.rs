use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::{SmtpConfig, SmtpTls};
use crate::error::{Error, Result};

/// Outgoing mail over SMTP. Only constructed when `SMTP_HOST` is set.
#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl Mailer {
    pub fn new(cfg: &SmtpConfig) -> Result<Self> {
        let builder = match cfg.tls {
            SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host),
            SmtpTls::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host),
            SmtpTls::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)),
        }
        .map_err(|e| Error::internal(format!("SMTP: {e}")))?;
        let mut builder = builder.port(cfg.port);
        if let (Some(user), Some(pass)) = (&cfg.username, &cfg.password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from: cfg.from.clone(),
        })
    }

    pub async fn send(&self, to: &str, subject: &str, body: String) -> Result<()> {
        let message = Message::builder()
            .from(self.from.parse().map_err(|e| Error::internal(format!("from: {e}")))?)
            .to(to.parse().map_err(|_| Error::invalid("Ungültige Empfängeradresse."))?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .map_err(|e| Error::internal(format!("mail: {e}")))?;
        self.transport
            .send(message)
            .await
            .map_err(|e| Error::internal(format!("SMTP send: {e}")))?;
        Ok(())
    }
}
