mod testnet;

mod integration {
    use litesvm::LiteSVM;
    use solana_instruction::{AccountMeta, Instruction};
    use solana_keypair::Keypair;
    use solana_message::Message;
    use solana_pubkey::Pubkey;
    use solana_signer::Signer;
    use solana_transaction::Transaction;

    const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
    const PACKET_FEE_LAMPORTS: u64 = 10_000;
    const UPDATED_PACKET_FEE_LAMPORTS: u64 = 15_000;

    struct ReusableState {
        config_authority: Keypair,
        fee_withdraw_authority: Keypair,
        sender: Keypair,
        destination: Keypair,
        system_program: Pubkey,
        global_config: Pubkey,
        sender_vault: Pubkey,
    }

    fn program_id() -> Pubkey {
        Pubkey::from(crate::ID)
    }

    fn send(svm: &mut LiteSVM, label: &str, payer: &Keypair, instruction: Instruction) {
        let transaction = Transaction::new(
            &[payer],
            Message::new(&[instruction], Some(&payer.pubkey())),
            svm.latest_blockhash(),
        );
        let result = svm
            .send_transaction(transaction)
            .expect("Nik8 transaction succeeds");
        println!("{label} CUs consumed: {:?}", result.compute_units_consumed);
    }

    fn setup() -> (LiteSVM, ReusableState) {
        let mut svm = LiteSVM::new();
        let config_authority = Keypair::new_from_array([11; 32]);
        let fee_withdraw_authority = Keypair::new_from_array([12; 32]);
        let sender = Keypair::new_from_array([13; 32]);
        let destination = Keypair::new_from_array([14; 32]);
        let system_program: Pubkey = SYSTEM_PROGRAM.parse().expect("system program ID");

        for address in [
            config_authority.pubkey(),
            fee_withdraw_authority.pubkey(),
            sender.pubkey(),
            destination.pubkey(),
        ] {
            svm.airdrop(&address, 10_000_000_000)
                .expect("account is funded");
        }

        let program_bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/deploy/nik8_program.so"
        ))
        .expect("build the SBF program first: cargo build-sbf");
        svm.add_program(program_id(), &program_bytes)
            .expect("Nik8 program loads");

        let (global_config, _) =
            solana_address::Address::find_program_address(&[b"nik8_config"], &crate::ID);
        let (sender_vault, _) = solana_address::Address::find_program_address(
            &[b"nik8_user", sender.pubkey().as_ref()],
            &crate::ID,
        );

        (
            svm,
            ReusableState {
                config_authority,
                fee_withdraw_authority,
                sender,
                destination,
                system_program,
                global_config,
                sender_vault,
            },
        )
    }

    #[test]
    fn sender_packet_fee_lifecycle() {
        let (mut svm, state) = setup();
        let ReusableState {
            config_authority,
            fee_withdraw_authority,
            sender,
            destination,
            system_program,
            global_config,
            sender_vault,
        } = state;

        let mut initialize_data = vec![0];
        initialize_data.extend_from_slice(&PACKET_FEE_LAMPORTS.to_le_bytes());
        initialize_data.extend_from_slice(&fee_withdraw_authority.pubkey().to_bytes());
        send(
            &mut svm,
            "initialize",
            &config_authority,
            Instruction {
                program_id: program_id(),
                accounts: vec![
                    AccountMeta::new(config_authority.pubkey(), true),
                    AccountMeta::new(global_config, false),
                    AccountMeta::new_readonly(system_program, false),
                ],
                data: initialize_data,
            },
        );

        send(
            &mut svm,
            "create sender vault",
            &sender,
            Instruction {
                program_id: program_id(),
                accounts: vec![
                    AccountMeta::new(sender.pubkey(), true),
                    AccountMeta::new(sender_vault, false),
                    AccountMeta::new_readonly(system_program, false),
                ],
                data: vec![2],
            },
        );

        let mut update_fee_data = vec![4];
        update_fee_data.extend_from_slice(&UPDATED_PACKET_FEE_LAMPORTS.to_le_bytes());
        send(
            &mut svm,
            "update packet fee",
            &config_authority,
            Instruction {
                program_id: program_id(),
                accounts: vec![
                    AccountMeta::new_readonly(config_authority.pubkey(), true),
                    AccountMeta::new(global_config, false),
                ],
                data: update_fee_data,
            },
        );

        let vault_before_packet = svm
            .get_account(&sender_vault)
            .expect("sender vault was created")
            .lamports;

        let mut packet_data = vec![1, 1, 1];
        packet_data.extend_from_slice(&[9; 16]);
        packet_data.extend_from_slice(&destination.pubkey().to_bytes());
        packet_data.extend_from_slice(&0_u32.to_le_bytes());
        packet_data.extend_from_slice(&1_000_u64.to_le_bytes());
        packet_data.extend_from_slice(&[0xF8, 0xFF, 0xFE]);
        send(
            &mut svm,
            "send voice packet",
            &sender,
            Instruction {
                program_id: program_id(),
                accounts: vec![
                    AccountMeta::new(sender.pubkey(), true),
                    AccountMeta::new(sender_vault, false),
                    AccountMeta::new_readonly(global_config, false),
                    AccountMeta::new_readonly(system_program, false),
                ],
                data: packet_data,
            },
        );
        assert_eq!(
            svm.get_account(&sender_vault).unwrap().lamports,
            vault_before_packet + UPDATED_PACKET_FEE_LAMPORTS
        );

        let withdrawal = 4_000_u64;
        let destination_before = svm.get_account(&destination.pubkey()).unwrap().lamports;
        let mut withdraw_data = vec![3];
        withdraw_data.extend_from_slice(&withdrawal.to_le_bytes());
        send(
            &mut svm,
            "withdraw packet fees",
            &fee_withdraw_authority,
            Instruction {
                program_id: program_id(),
                accounts: vec![
                    AccountMeta::new_readonly(fee_withdraw_authority.pubkey(), true),
                    AccountMeta::new_readonly(global_config, false),
                    AccountMeta::new(sender_vault, false),
                    AccountMeta::new(destination.pubkey(), false),
                ],
                data: withdraw_data,
            },
        );
        assert_eq!(
            svm.get_account(&sender_vault).unwrap().lamports,
            vault_before_packet + UPDATED_PACKET_FEE_LAMPORTS - withdrawal
        );
        assert_eq!(
            svm.get_account(&destination.pubkey()).unwrap().lamports,
            destination_before + withdrawal
        );
    }
}
