/// ═══════════════════════════════════════════════════════════════════════════
///  POLYMARKET — HERRAMIENTA DE CONFIGURACIÓN INICIAL
///  Ejecutar UNA SOLA VEZ para generar las API keys L2.
///
///  Uso:
///    cargo run --bin setup
///
///  Qué hace:
///    1. Lee POLYMARKET_PRIVATE_KEY desde .env
///    2. Deriva la dirección Ethereum de esa clave
///    3. Llama a POST /auth/api-key en el CLOB de Polymarket
///       (idempotente: devuelve las existentes si ya hay unas)
///    4. Imprime los tres valores que debes copiar a tu .env:
///       CLOB_API_KEY / CLOB_API_SECRET / CLOB_API_PASSPHRASE
///
///  Después de copiar los valores, este binario ya no es necesario.
///  Puedes correr el dashboard con: cargo run
/// ═══════════════════════════════════════════════════════════════════════════
use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result};
use polymarket_client_sdk::PRIVATE_KEY_VAR; // = "POLYMARKET_PRIVATE_KEY"
use polymarket_client_sdk::clob::{Client, Config};
use secrecy::ExposeSecret as _;

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Cargar .env ────────────────────────────────────────────────────
    if let Err(e) = dotenvy::dotenv() {
        eprintln!("Aviso: .env no encontrado ({e}). Usando variables del entorno.");
    }

    // ── 2. Leer la clave privada ──────────────────────────────────────────
    let private_key = std::env::var(PRIVATE_KEY_VAR)
        .or_else(|_| std::env::var("PRIVATE_KEY"))
        .context(format!(
            "❌  Variable `{PRIVATE_KEY_VAR}` no encontrada.\n\
             Asegúrate de que tu .env contiene:\n\
             {PRIVATE_KEY_VAR}=0xTU_CLAVE_PRIVADA"
        ))?;

    // ── 3. Construir el signer Ethereum (alloy) ───────────────────────────
    //
    // PrivateKeySigner implementa alloy::signers::Signer, el trait requerido
    // por authentication_builder() y create_or_derive_api_key().
    // with_chain_id(POLYGON) añade el chain ID 137 a las firmas EIP-155.
    let signer = PrivateKeySigner::from_str(&private_key)
        .context("❌  Clave privada inválida (se esperan 64 hex chars, con o sin 0x)")?
        .with_chain_id(Some(polymarket_client_sdk::POLYGON));

    let wallet_addr = format!("{:#x}", signer.address());
    println!();
    println!("  Wallet detectada : {wallet_addr}");
    println!("  Red              : Polygon (chain ID 137)");
    println!();

    // ── 4. Crear cliente CLOB sin autenticar ──────────────────────────────
    //
    // create_or_derive_api_key() está disponible en Client<Unauthenticated>.
    // Firma el request L1 con el signer para probar ownership de la wallet,
    // y el servidor devuelve (o crea) las credenciales L2 asociadas.
    let client = Client::new("https://clob.polymarket.com", Config::default())
        .context("❌  No se pudo crear el cliente CLOB")?;

    println!("  Conectando con Polymarket CLOB...");
    println!("  Llamando a create_or_derive_api_key()  (puede tardar unos segundos)");
    println!();

    // ── 5. Obtener / crear las credenciales L2 ────────────────────────────
    //
    // - Si la wallet NO tiene API keys: las crea (POST /auth/api-key)
    // - Si la wallet YA tiene API keys: las devuelve (GET /auth/derive-api-key)
    // El nonce=None usa el valor por defecto (0).
    let credentials = client
        .create_or_derive_api_key(&signer, None)
        .await
        .context(
            "❌  Error al crear/derivar las API keys.\n\
             Comprueba que la clave privada es correcta y que tienes\n\
             conexión a internet (clob.polymarket.com).",
        )?;

    // ── 6. Mostrar los valores ────────────────────────────────────────────
    //
    // SecretString oculta los valores en Debug/Display para evitar leaks
    // accidentales en logs. ExposeSecret::expose_secret() los revela aquí
    // de forma explícita e intencional.
    let api_key        = credentials.key().to_string();
    let api_secret     = credentials.secret().expose_secret().to_string();
    let api_passphrase = credentials.passphrase().expose_secret().to_string();

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║         CREDENCIALES GENERADAS — CÓPIALAS A TU .env         ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                              ║");
    println!("║  CLOB_API_KEY={api_key:<47}║");
    println!("║  CLOB_API_SECRET={api_secret:<45}║");
    println!("║  CLOB_API_PASSPHRASE={api_passphrase:<41}║");
    println!("║                                                              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("  ✅  Pasos siguientes:");
    println!("      1. Abre tu archivo .env");
    println!("      2. Pega las tres líneas de arriba");
    println!("      3. Guarda el archivo");
    println!("      4. Corre el dashboard:  cargo run");
    println!();
    println!("  ⚠️   SEGURIDAD:");
    println!("      • NUNCA compartas CLOB_API_SECRET ni CLOB_API_PASSPHRASE");
    println!("      • Asegúrate de que .gitignore incluye: .env");
    println!("      • Si sospechas que las claves están comprometidas,");
    println!("        corre este setup de nuevo para regenerarlas.");
    println!();

    Ok(())
}
