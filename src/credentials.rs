/// Gestión segura de credenciales.
///
/// Variables requeridas en .env:
///   POLYMARKET_PRIVATE_KEY  — clave privada Ethereum (hex, con o sin 0x)
///   CLOB_API_KEY            — API key L2 del CLOB
///   CLOB_API_SECRET         — API secret L2 del CLOB
///   CLOB_API_PASSPHRASE     — passphrase L2 del CLOB
///
/// La dirección de la wallet se DERIVA de la clave privada usando alloy;
/// nunca se almacena en texto plano ni se imprime en los logs.
use std::str::FromStr as _;

use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer as _;
use anyhow::{Context, Result};
use polymarket_client_sdk::PRIVATE_KEY_VAR; // = "POLYMARKET_PRIVATE_KEY"

// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClobCredentials {
    /// Clave privada en hex — guardada en memoria sólo para recrear el signer
    /// cuando sea necesario firmar órdenes. NO loguear.
    pub(crate) private_key: String,

    /// Credenciales L2 para llamadas autenticadas (POST /order, /cancel, etc.)
    /// No se usan en el dashboard de lectura; reservadas para order placement.
    #[allow(dead_code)]
    pub api_key:        String,
    #[allow(dead_code)]
    pub api_secret:     String,
    #[allow(dead_code)]
    pub api_passphrase: String,

    /// Dirección Ethereum (checksummed) derivada de la clave privada
    pub wallet_address: String,
}

impl ClobCredentials {
    /// Carga las credenciales desde variables de entorno.
    /// Llama a `dotenvy::dotenv()` ANTES de invocar este método.
    pub fn from_env() -> Result<Self> {
        // Intentamos primero el nombre del SDK; si no, el alias corto
        let private_key = std::env::var(PRIVATE_KEY_VAR)
            .or_else(|_| std::env::var("PRIVATE_KEY"))
            .context(format!(
                "Falta la variable de entorno `{PRIVATE_KEY_VAR}` (o `PRIVATE_KEY`)"
            ))?;

        let api_key = std::env::var("CLOB_API_KEY")
            .context("Falta `CLOB_API_KEY` en .env")?;

        let api_secret = std::env::var("CLOB_API_SECRET")
            .context("Falta `CLOB_API_SECRET` en .env")?;

        let api_passphrase = std::env::var("CLOB_API_PASSPHRASE")
            .context("Falta `CLOB_API_PASSPHRASE` en .env")?;

        let wallet_address = derive_address(&private_key)
            .context("No se pudo derivar la dirección desde POLYMARKET_PRIVATE_KEY")?;

        Ok(Self {
            private_key,
            api_key,
            api_secret,
            api_passphrase,
            wallet_address,
        })
    }

    /// Reconstruye el signer alloy. Úsalo en el worker para autenticar el cliente.
    pub fn build_signer(&self) -> Result<PrivateKeySigner> {
        let signer = PrivateKeySigner::from_str(&self.private_key)
            .context("Clave privada inválida")?
            .with_chain_id(Some(polymarket_client_sdk::POLYGON));
        Ok(signer)
    }

    /// Dirección truncada para mostrar en UI: `0x1234...abcd`
    pub fn display_address(&self) -> String {
        let a = &self.wallet_address;
        if a.len() >= 12 {
            format!("{}...{}", &a[..6], &a[a.len() - 4..])
        } else {
            a.clone()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

fn derive_address(private_key_hex: &str) -> Result<String> {
    let signer = PrivateKeySigner::from_str(private_key_hex)
        .context("Formato de clave privada inválido (se espera hex de 32 bytes)")?;
    Ok(format!("{:#x}", signer.address()))
}
