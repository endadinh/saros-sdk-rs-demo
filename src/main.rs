use anyhow::Result;
use jupiter_amm_interface::SwapMode;
use saros_sdk::{
    math::{
        fees::{
            compute_transfer_amount_for_expected_output, compute_transfer_fee, TokenTransferFee,
        },
        swap_manager::get_swap_result,
    },
    state::{
        bin_array::{BinArray, BinArrayPair},
        pair::Pair,
    },
    utils::helper::{self, is_swap_for_y},
};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{program_pack::Pack, pubkey::Pubkey};

pub const RPC_URL: &str = "https://api.devnet.solana.com";

//  pair id on devnet
pub const PAIR: &str = "FvKuEuRyfDZ8catHJznC7heKLkC1uopRaaKMDY1Nym2T";

pub struct SarosDlmm {
    pub client: RpcClient,
    pub program_id: Pubkey,
    pub pool: Pubkey,
    pub pair: Pair,
}
pub struct QuoteParams {
    pub amount: u64,
    pub swap_mode: SwapMode,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
}
pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
}

impl SarosDlmm {
    pub fn new(program_id: Pubkey, pair: Pubkey) -> Self {
        let pair_state = RpcClient::new(RPC_URL.to_string())
            .get_account(&pair)
            .expect("Failed to fetch account")
            .data;

        let pair_account = Pair::unpack(&pair_state).expect("Failed to unpack account");

        Self {
            client: RpcClient::new(RPC_URL.to_string()),
            program_id,
            pool: pair,
            pair: pair_account,
        }
    }

    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote> {
        let QuoteParams {
            amount,
            swap_mode,
            input_mint,
            ..
        } = *quote_params;
        let mut pair = self.pair.clone();

        let (bin_array_lower, _) =
            helper::get_bin_array_lower(self.pair.bin_array_index(), &self.pool, &self.program_id);
        let (bin_array_upper, _) =
            helper::get_bin_array_upper(self.pair.bin_array_index(), &self.pool, &self.program_id);

        let slot = self.client.get_slot()?;
        let block_timestamp = self.client.get_block_time(slot)? as u64;
        let swap_for_y = is_swap_for_y(input_mint, self.pair.token_mint_x);

        let mut token_transfer_fee: TokenTransferFee = TokenTransferFee::default();

        let token_mint_x_data = self.client.get_account(&self.pair.token_mint_x)?;
        let token_mint_y_data = self.client.get_account(&self.pair.token_mint_y)?;

        let epoch = self.client.get_epoch_info()?.epoch;

        let bin_lower_account = self.client.get_account(&bin_array_lower)?;
        let bin_upper_account = self.client.get_account(&bin_array_upper)?;

        let bin_lower_data = BinArray::unpack(&bin_lower_account.data)?;
        let bin_upper_data = BinArray::unpack(&bin_upper_account.data)?;

        let bin_array = BinArrayPair::merge(bin_lower_data, bin_upper_data)?;

        TokenTransferFee::new(
            &mut token_transfer_fee,
            token_mint_x_data.data.as_ref(),
            &token_mint_x_data.owner,
            &token_mint_y_data.data.as_ref(),
            &token_mint_y_data.owner,
            epoch,
        )?;

        let (mint_in, epoch_transfer_fee_in, epoch_transfer_fee_out) = if swap_for_y {
            (
                self.pair.token_mint_x,
                token_transfer_fee.epoch_transfer_fee_x,
                token_transfer_fee.epoch_transfer_fee_y,
            )
        } else {
            (
                self.pair.token_mint_y,
                token_transfer_fee.epoch_transfer_fee_y,
                token_transfer_fee.epoch_transfer_fee_x,
            )
        };

        let (amount_in, amount_out, fee_amount) = match swap_mode {
            SwapMode::ExactIn => {
                let (amount_in_after_transfer_fee, _) =
                    compute_transfer_fee(epoch_transfer_fee_in, amount)?;

                let (amount_out, fee_amount) = get_swap_result(
                    &mut pair,
                    bin_array,
                    amount_in_after_transfer_fee,
                    swap_for_y,
                    swap_mode,
                    block_timestamp,
                )?;

                let (amount_out_after_transfer_fee, _) =
                    compute_transfer_fee(epoch_transfer_fee_out, amount_out)?;

                (amount, amount_out_after_transfer_fee, fee_amount)
            }
            SwapMode::ExactOut => {
                let (amount_out_before_transfer_fee, _) =
                    compute_transfer_amount_for_expected_output(epoch_transfer_fee_out, amount)?;

                let (amount_in, fee_amount) = get_swap_result(
                    &mut pair,
                    bin_array,
                    amount_out_before_transfer_fee,
                    swap_for_y,
                    swap_mode,
                    block_timestamp,
                )?;

                let (amount_in_before_transfer_fee, _) =
                    compute_transfer_amount_for_expected_output(epoch_transfer_fee_in, amount_in)?;

                let (amount_out_after_transfer_fee, _) =
                    compute_transfer_fee(epoch_transfer_fee_out, amount)?;

                (
                    amount_in_before_transfer_fee,
                    amount_out_after_transfer_fee,
                    fee_amount,
                )
            }
        };

        Ok(Quote {
            in_amount: amount_in,
            out_amount: amount_out,
            fee_amount,
            fee_mint: mint_in,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {

    // // Pair pubkey
    let pair_key = Pubkey::try_from(PAIR)?;

    let saros_dlmm = SarosDlmm::new(
        saros::ID, 
        pair_key,
    );

    let quote = saros_dlmm.quote(&QuoteParams {
        amount: 1_000_000, // 1 token in base units
        swap_mode: SwapMode::ExactIn,
        input_mint: saros_dlmm.pair.token_mint_x,  // token X
        output_mint: saros_dlmm.pair.token_mint_y, // token Y
    })?;

    println!(
        "Quote: in_amount: {}, out_amount: {}, fee_amount: {}, fee_mint: {}",
        quote.in_amount, quote.out_amount, quote.fee_amount, quote.fee_mint
    );

    Ok(())
}
