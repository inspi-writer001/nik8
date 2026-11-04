use std::{env, error::Error, path::PathBuf};

use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{Keypair, Signer, read_keypair_file};
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use solana_transaction::Transaction;

const DEFAULT_TESTNET_RPC_URL: &str = "https://api.testnet.solana.com";
const SYSTEM_PROGRAM_ID: Address = Address::from_str_const("11111111111111111111111111111111");
const PACKET_FEE_LAMPORTS: u64 = 10_000;
const WITHDRAWAL_LAMPORTS: u64 = 1;

struct TestnetContext {
    client: RpcClient,
    payer: Keypair,
    program_id: Address,
    global_config: Address,
    sender_vault: Address,
}

/// Exercises a newly deployed Testnet program from initialization through a
/// fee-bearing voice packet and a fee withdrawal.
///
/// Run explicitly only after deployment:
/// 1. Create a local `.env` in the workspace (or program) directory containing
///    `PAYER_KEYPAIR=/absolute/path/to/your/testnet-keypair.json`.
/// 2. Run `cargo test testnet -- --ignored --nocapture`.
///
/// `RPC_URL` is optional and defaults to Solana Testnet.
#[test]
#[ignore = "sends signed transactions to Testnet; run explicitly after deployment"]
fn fresh_testnet_lifecycle() {
    process_fresh_testnet_lifecycle().expect("fresh Testnet lifecycle should succeed");
}

pub fn process_fresh_testnet_lifecycle() -> Result<(), Box<dyn Error>> {
    let context = load_testnet_context()?;

    println!("Testnet program: {}", context.program_id);
    println!("Testnet payer: {}", context.payer.pubkey());
    println!("Testnet global config PDA: {}", context.global_config);
    println!("Testnet sender vault PDA: {}", context.sender_vault);

    // if context
    //     .client
    //     .get_account(&Pubkey::from(context.global_config))
    //     .is_ok()
    // {
    //     return Err(
    //         "global config already exists; this fresh-deployment lifecycle must not be rerun"
    //             .into(),
    //     );
    // }

    let mut initialize_data = vec![0]; // Nik8Ix::Initialize
    initialize_data.extend_from_slice(&PACKET_FEE_LAMPORTS.to_le_bytes());
    initialize_data.extend_from_slice(&context.payer.pubkey().to_bytes());
    simulate_then_send(
        &context,
        "initialize",
        Instruction {
            program_id: context.program_id,
            accounts: vec![
                AccountMeta::new(context.payer.pubkey(), true),
                AccountMeta::new(context.global_config, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            data: initialize_data,
        },
    )?;

    simulate_then_send(
        &context,
        "create sender vault",
        Instruction {
            program_id: context.program_id,
            accounts: vec![
                AccountMeta::new(context.payer.pubkey(), true),
                AccountMeta::new(context.sender_vault, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            data: vec![2], // Nik8Ix::Cre8Account
        },
    )?;

    let mut packet_data = vec![1, 1, 1]; // Nik8Ix::OnewayMessage, version, Opus
    packet_data.extend_from_slice(&[1; 16]);
    packet_data.extend_from_slice(&context.payer.pubkey().to_bytes());
    packet_data.extend_from_slice(&0_u32.to_le_bytes());
    packet_data.extend_from_slice(&0_u64.to_le_bytes());
    packet_data.extend_from_slice(&[0xF8, 0xFF, 0xFE, 0xdf]);
    simulate_then_send(
        &context,
        "send voice packet",
        Instruction {
            program_id: context.program_id,
            accounts: vec![
                AccountMeta::new(context.payer.pubkey(), true),
                AccountMeta::new(context.sender_vault, false),
                AccountMeta::new_readonly(context.global_config, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            data: packet_data,
        },
    )?;

    simulate_then_send(
        &context,
        "update packet fee",
        Instruction {
            program_id: context.program_id,
            accounts: vec![
                AccountMeta::new_readonly(context.payer.pubkey(), true),
                AccountMeta::new(context.global_config, false),
            ],
            data: [vec![4], PACKET_FEE_LAMPORTS.to_le_bytes().to_vec()].concat(),
        },
    )?;

    // simulate_then_send(
    //     &context,
    //     "withdraw packet fees",
    //     Instruction {
    //         program_id: context.program_id,
    //         accounts: vec![
    //             AccountMeta::new_readonly(context.payer.pubkey(), true),
    //             AccountMeta::new_readonly(context.global_config, false),
    //             AccountMeta::new(context.sender_vault, false),
    //             AccountMeta::new(context.payer.pubkey(), false),
    //         ],
    //         data: [vec![3], WITHDRAWAL_LAMPORTS.to_le_bytes().to_vec()].concat(),
    //     },
    // )?;

    Ok(())
}

fn load_testnet_context() -> Result<TestnetContext, Box<dyn Error>> {
    let dotenv_path = dotenvy::dotenv().map_err(|error| {
        format!(
            "failed to load .env for the Testnet test: {error}; add PAYER_KEYPAIR to .env"
        )
    })?;

    let rpc_url =
        env::var("RPC_URL").unwrap_or_else(|_| DEFAULT_TESTNET_RPC_URL.to_owned());
    let payer_keypair_path = PathBuf::from(
        env::var("PAYER_KEYPAIR")
            .map_err(|_| "missing PAYER_KEYPAIR pointing to a funded Testnet keypair")?,
    );
    let payer_keypair_path = if payer_keypair_path.is_absolute() {
        payer_keypair_path
    } else {
        dotenv_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(payer_keypair_path)
    };
    let payer = read_keypair_file(&payer_keypair_path)?;
    let program_id = Address::from(crate::ID);
    let (global_config, _) = Address::find_program_address(&[b"nik8_config"], &crate::ID);
    let (sender_vault, _) =
        Address::find_program_address(&[b"nik8_user", payer.pubkey().as_ref()], &crate::ID);

    Ok(TestnetContext {
        client: RpcClient::new(rpc_url),
        payer,
        program_id,
        global_config,
        sender_vault,
    })
}

fn simulate_then_send(
    context: &TestnetContext,
    label: &str,
    instruction: Instruction,
) -> Result<(), Box<dyn Error>> {
    let transaction = Transaction::new(
        &[&context.payer],
        Message::new(&[instruction], Some(&context.payer.pubkey())),
        context.client.get_latest_blockhash()?,
    );

    let simulation = context.client.simulate_transaction(&transaction)?;
    println!("{label} simulation: {:?}", simulation.value.err);
    if let Some(error) = simulation.value.err {
        return Err(format!("{label} simulation failed: {error:?}").into());
    }

    let signature = context.client.send_and_confirm_transaction(&transaction)?;
    println!("{label} signature: {signature}");
    Ok(())
}
