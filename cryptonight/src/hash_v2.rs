use cnaes::AES_BLOCK_SIZE;

use crate::{
    cnaes,
    slow_hash::{Variant, MEMORY},
    util::{subarray, subarray_copy},
};

fn block_to_u64le(block: &[u8; AES_BLOCK_SIZE]) -> [u64; 2] {
    [
        u64::from_le_bytes(subarray_copy(block, 0)),
        u64::from_le_bytes(subarray_copy(block, 8)),
    ]
}

/// Original C code:
/// <https://github.com/monero-project/monero/blob/v0.18.3.4/src/crypto/slow-hash.c#L217-L254>
/// If we kept the C code organization, this function would be in `slow_hash.rs`, but it's
/// here in the rust code to keep the `slow_hash.rs` file size manageable.
pub(crate) fn variant2_shuffle_add(
    c1: &mut [u8; AES_BLOCK_SIZE],
    a: &[u8; AES_BLOCK_SIZE],
    b: &[u8; AES_BLOCK_SIZE * 2],
    long_state: &mut [u8; MEMORY],
    offset: usize,
    variant: Variant,
) {
    if variant == Variant::V2 || variant == Variant::R {
        let chunk1_start = offset ^ 0x10;
        let chunk2_start = offset ^ 0x20;
        let chunk3_start = offset ^ 0x30;

        let chunk1: &[u8; AES_BLOCK_SIZE] = subarray(long_state, chunk1_start);
        let chunk2: &[u8; AES_BLOCK_SIZE] = subarray(long_state, chunk2_start);
        let chunk3: &[u8; AES_BLOCK_SIZE] = subarray(long_state, chunk3_start);

        let mut chunk1_old = block_to_u64le(chunk1);
        let chunk2_old = block_to_u64le(chunk2);
        let chunk3_old = block_to_u64le(chunk3);

        let b1 = block_to_u64le(subarray(b, 16));

        let chunk1 = &mut long_state[chunk1_start..chunk1_start + 16];
        chunk1[0..8].copy_from_slice(&(chunk3_old[0].wrapping_add(b1[0]).to_le_bytes()));
        chunk1[8..16].copy_from_slice(&(chunk3_old[1].wrapping_add(b1[1]).to_le_bytes()));

        let a0 = block_to_u64le(a);

        let chunk3 = &mut long_state[chunk3_start..chunk3_start + 16];
        chunk3[0..8].copy_from_slice(&(chunk2_old[0].wrapping_add(a0[0])).to_le_bytes());
        chunk3[8..16].copy_from_slice(&(chunk2_old[1].wrapping_add(a0[1])).to_le_bytes());

        let b0 = block_to_u64le(subarray(b, 0));
        let chunk2 = &mut long_state[chunk2_start..chunk2_start + 16];
        chunk2[0..8].copy_from_slice(&(chunk1_old[0].wrapping_add(b0[0])).to_le_bytes());
        chunk2[8..16].copy_from_slice(&(chunk1_old[1].wrapping_add(b0[1])).to_le_bytes());

        if variant == Variant::R {
            let mut out_copy = block_to_u64le(c1);

            chunk1_old[0] ^= chunk2_old[0];
            chunk1_old[1] ^= chunk2_old[1];
            out_copy[0] ^= chunk3_old[0];
            out_copy[1] ^= chunk3_old[1];
            out_copy[0] ^= chunk1_old[0];
            out_copy[1] ^= chunk1_old[1];

            c1[0..8].copy_from_slice(&out_copy[0].to_le_bytes());
            c1[8..16].copy_from_slice(&out_copy[1].to_le_bytes());
        }
    }
}

#[expect(clippy::cast_sign_loss)]
#[expect(clippy::cast_precision_loss)]
#[expect(clippy::cast_possible_truncation)]
pub(crate) fn variant2_integer_math_sqrt(sqrt_input: u64) -> u64 {
    // Get an approximation using floating point math
    let mut sqrt_result =
        ((sqrt_input as f64 + 18_446_744_073_709_552_000.0).sqrt() * 2.0 - 8589934592.0) as u64;

    // Fixup the edge cases to get the exact integer result. For more information,
    // see: https://github.com/monero-project/monero/blob/v0.18.3.3/src/crypto/variant2_int_sqrt.h#L65-L152
    let sqrt_div2 = sqrt_result >> 1;
    let lsb = sqrt_result & 1;
    let r2 = sqrt_div2
        .wrapping_mul(sqrt_div2 + lsb)
        .wrapping_add(sqrt_result << 32);

    if r2.wrapping_add(lsb) > sqrt_input {
        sqrt_result = sqrt_result.wrapping_sub(1);
    }
    if r2.wrapping_add(1 << 32) < sqrt_input.wrapping_sub(sqrt_div2) {
        // Not sure that this is possible. I tried writing a test program
        // to search subsets of u64 for a value that can trigger this
        // branch, but couldn't find anything. The Go implementation came
        // to the same conclusion:
        // https://github.com/Equim-chan/cryptonight/blob/v0.3.0/arith_ref.go#L39-L45
        sqrt_result = sqrt_result.wrapping_add(1);
    }

    sqrt_result
}

/// Original C code:
/// <https://github.com/monero-project/monero/blob/v0.18.3.4/src/crypto/slow-hash.c#L277-L283>
pub(crate) fn variant2_integer_math(
    c2: &mut [u8; 8],
    c1: &[u8; AES_BLOCK_SIZE],
    division_result: &mut u64,
    sqrt_result: &mut u64,
    variant: Variant,
) {
    const U32_MASK: u64 = u32::MAX as u64;

    if variant == Variant::V2 {
        let tmpx = *division_result ^ (*sqrt_result << 32);
        *c2 = (u64::from_le_bytes(*c2) ^ tmpx).to_le_bytes();

        let c1_64 = block_to_u64le(c1);
        let mut divisor = c1_64[0];
        let dividend = c1_64[1];

        divisor = ((divisor + ((*sqrt_result << 1) & U32_MASK)) | 0x80000001) & U32_MASK;
        *division_result =
            ((dividend / divisor) & U32_MASK).wrapping_add((dividend % divisor) << 32);

        let sqrt_input = c1_64[0].wrapping_add(*division_result);
        *sqrt_result = variant2_integer_math_sqrt(sqrt_input);
    }
}

#[cfg(test)]
mod tests {
    use digest::Digest;
    use groestl::Groestl256;

    use super::*;
    use crate::util::{hex_to_array, subarray_mut};

    #[test]
    fn test_variant2_integer_math() {
        let test = |c2_hex: &str,
                    c1_hex: &str,
                    division_result: u64,
                    sqrt_result: u64,
                    c2_hex_end: &str,
                    division_result_end: u64,
                    sqrt_result_end: u64| {
            let mut c2: [u8; 16] = hex_to_array(c2_hex);
            let c1: [u8; 16] = hex_to_array(c1_hex);
            let mut division_result = division_result;
            let mut sqrt_result = sqrt_result;

            variant2_integer_math(
                subarray_mut(&mut c2, 0),
                &c1,
                &mut division_result,
                &mut sqrt_result,
                Variant::V2,
            );

            assert_eq!(hex::encode(c2), c2_hex_end);
            assert_eq!(division_result, division_result_end);
            assert_eq!(sqrt_result, sqrt_result_end);
        };
        test(
            "8b4d610801fe2049741c4cf1a11912d5",
            "ef9d5925ad73f044f6310bce80f333a4",
            1992885167645223034,
            15156498822412360757,
            "f125c247b4040b0e741c4cf1a11912d5",
            11701596267494179432,
            3261805857,
        );
        test(
            "540ac7dbbddf5b93fdc90f999408b7ad",
            "10d2c1fdcbf7246e8623a3d946bdf422",
            6226440187041759132,
            1708636566,
            "c83510b077a4e4a0fdc90f999408b7ad",
            6478148604080708997,
            2875078897,
        );
        test(
            "0df28c3c3570ae3b68dc9d6c5a486ed7",
            "a5fba99aa63fa032acf1bd65ff4df3f2",
            11107069037757228366,
            2924318811,
            "4397ce171fdcc70f68dc9d6c5a486ed7",
            7549089838000449301,
            2299293038,
        );
        test(
            "bfe14f97a968a35d0dcd6890a03c2913",
            "d4a80e16ad64e3a0624a795c7b349c8a",
            15584044376391133794,
            276486141,
            "dd4bf8759e1a9c950dcd6890a03c2913",
            4771913259875991617,
            3210383690,
        );

        test(
            "820692e47779a9aabf0621e52a142468",
            "df61b75f65251ee61828166e565336a9",
            3269677112081011360,
            1493829760,
            "2254426ff54bc3debf0621e52a142468",
            2626216843989114230,
            175440206,
        );
        test(
            "0b364e61de218e00e83c4073b39daa2e",
            "cc463d4543eb430d08efedf2be86e322",
            7096668609104405526,
            713261042,
            "1d521b6fac307148e83c4073b39daa2e",
            8234613052379859783,
            1924288792,
        );
        test(
            "bd8fff861f6315c2be812b64cbdcf646",
            "38d1e323d9dc282fa5e68f2ecbdcb950",
            9545374795048279136,
            271106137,
            "dd532ef48b584a56be812b64cbdcf646",
            2790373411402251888,
            1336862722,
        );
        test(
            "ed57e73448f357bf04dc831d5e8fd848",
            "a5dcd0971e6ded60d4d98c03cd8ba205",
            5991074580974163125,
            2246952057,
            "580331e9a7a59e6904dc831d5e8fd848",
            7395390641079862703,
            2868947253,
        );
        test(
            "07ea0ffc6e182a7e97853f82e459d625",
            "7e403d950f4adc97b90140875c33d65f",
            8836830558353968711,
            1962375668,
            "40a40e3f08db7f7097853f82e459d625",
            5478469695216926448,
            3219877666,
        );
        test(
            "b77688d600a356077021e2333ee3def4",
            "7a9f061760287a69b57f365163fb9dac",
            3127636279441542418,
            1585025819,
            "a5bb34d8bba848727021e2333ee3def4",
            3683326568856788118,
            2315202244,
        );
        test(
            "a246a7f62b7e3d9a0b5ac66166bfcba3",
            "23329476afdbd46d3be9d3ccc9011c11",
            12123559059253265496,
            819016365,
            "fac2e5d23dc4d3020b5ac66166bfcba3",
            4214751652441358299,
            2469122821,
        );
        test(
            "3e1abb8109c688405cd6c866cbdb3e13",
            "b4c10bf5e06c069928afa173f62d5017",
            7368515032603121941,
            2312559799,
            "2b43d451df231caf5cd6c866cbdb3e13",
            1324536149240623108,
            2509236669,
        );
        test(
            "a31260db7c73f249b5fbc182ae7fcc8e",
            "b4214755b0003e4c82d03f80d8a06bed",
            1904095218141907119,
            92928147,
            "0c5abeec6c3f1756b5fbc182ae7fcc8e",
            9883090335304272258,
            3041688469,
        );
        test(
            "e3d0bc3e619f577a1eea5adba205e494",
            "cd8040848aae39104c310c1fa0eed9b8",
            4873400164336079541,
            2436984787,
            "56c22935133bb7a81eea5adba205e494",
            8226478499779865232,
            1963241245,
        );
        test(
            "f22ac244fd17cf5e3ec21bece2581a2d",
            "785152f272ffa9514ef2ae0bed5cbaa7",
            6386228481616770937,
            1413583152,
            "8bddfda13af62e523ec21bece2581a2d",
            9654977853452823978,
            3069608655,
        );
        test(
            "37b3921988d9df1b38b04dc1db01a41b",
            "054b87f38d203eddb16d458048f3b97b",
            5592059432235016971,
            2670380708,
            "3c10afec40e36fc938b04dc1db01a41b",
            2475375116655310772,
            3553266751,
        );
        test(
            "cfd4afb021e526d9cbd4720cc47c4ce2",
            "a2e3e7fe936c2b38e3708965f2dfc586",
            11958325643570725319,
            825185219,
            "0895d52d3237fd4dcbd4720cc47c4ce2",
            2253955666499039951,
            1359567468,
        );
        test(
            "55d2ea9570994bc0aeaf6a3189bf0b4a",
            "9d102c34665382dfd36e39a67e07b8aa",
            10171590341391886242,
            541577843,
            "f7f59fbe85f4246daeaf6a3189bf0b4a",
            6907584596503955220,
            1004462004,
        );
        test(
            "bf32b60d6bbaa87cececd577f2ad15d8",
            "9a8471b2b72e9d39cd2d2cb124aa270a",
            9778648685358392468,
            469385479,
            "2b9696774746e6e0ececd577f2ad15d8",
            4910280747850874346,
            1899784302,
        );
        test(
            "d70ac5de7a390e2a735726324d0b52b5",
            "6cf5b75b005599047972995ffbe34101",
            2318211298357120319,
            1093372020,
            "e8871a66ea410e4b735726324d0b52b5",
            14587709575956469579,
            2962700286,
        );
        test(
            "412f463e5143eace451dcb2a2efd8022",
            "38ed251c7915236b2aca4ea995b861c9",
            10458537212399393571,
            621387691,
            "623403e9d4ecc77a451dcb2a2efd8022",
            12914179687381327414,
            495045866,
        );
    }

    #[test]
    fn test_variant2_integer_math_sqrt() {
        // Edge case values taken from here:
        // https://github.com/monero-project/monero/blob/v0.18.3.3/src/crypto/variant2_int_sqrt.h#L33-L43
        let test_cases = [
            (0, 0),
            (1 << 32, 0),
            ((1 << 32) + 1, 1),
            (1 << 50, 262140),
            ((1 << 55) + 20963331, 8384515),
            ((1 << 55) + 20963332, 8384516),
            ((1 << 62) + 26599786, 1013904242),
            ((1 << 62) + 26599787, 1013904243),
            (u64::MAX, 3558067407),
        ];

        for &(input, expected) in &test_cases {
            assert_eq!(
                variant2_integer_math_sqrt(input),
                expected,
                "input = {input}"
            );
        }
    }

    #[test]
    fn test_variant2_shuffle_add() {
        let test = |c1_hex: &str,
                    a_hex: &str,
                    b_hex: &str,
                    offset: usize,
                    variant: Variant,
                    c1_hex_end: &str,
                    long_state_end_hash: &str| {
            let mut c1: [u8; AES_BLOCK_SIZE] = hex_to_array(c1_hex);
            let a: [u8; AES_BLOCK_SIZE] = hex_to_array(a_hex);
            let b: [u8; AES_BLOCK_SIZE * 2] = hex_to_array(b_hex);

            let mut long_state = vec![0_u8; MEMORY];
            let long_state: &mut [u8; MEMORY] = subarray_mut(&mut long_state, 0);
            for (i, byte) in long_state.iter_mut().enumerate() {
                *byte = u8::try_from(i & 0xFF).unwrap();
            }

            variant2_shuffle_add(&mut c1, &a, &b, long_state, offset, variant);
            assert_eq!(hex::encode(c1), c1_hex_end);
            let hash = Groestl256::digest(long_state.as_slice());
            assert_eq!(hex::encode(hash), long_state_end_hash);
        };
        test(
            "d7143e3b6ffdeae4b2ceea30e9889c8a",
            "875fa34de3af48f15638bad52581ef4c",
            "b07d6f24f19434289b305525f094d8d7bd9d3c9bc956ac081d6186432a282a36",
            221056,
            Variant::R,
            "5795bcb8eb786c633a4760bb65051205",
            "26c32c4c2eeec340d62b88f5261d1a264c74240c2f8424c6e7101cf490e5772e",
        );
        test(
            "c7d6fe95ffd8d902d2cfc1883f7a2bc3",
            "bceb9d8cb71c2ac85c24129c94708e17",
            "4b3a589c187e26bea487b19ea36eb19e8369f4825642eb467c75bf07466b87ba",
            1960880,
            Variant::V2,
            "c7d6fe95ffd8d902d2cfc1883f7a2bc3",
            "2d4ddadd0e53a02797c62bf37d11bb2de73e6769abd834a81c1262752176a024",
        );
        test(
            "92ad41fc1596244e2e0f0bfed6555cef",
            "d1f0337e48c4f53742cedd78b6b33b67",
            "b17bce6c44e0f680aa0f0a28a4e3865b43cdd18644a383e7a9d2f17310e5b6aa",
            1306832,
            Variant::R,
            "427c932fc143f299f6d6d1250a888230",
            "984440e0b9f77f1159f09b13d2d455292d5a9b4095037f4e8ca2a0ed982bee8f",
        );
        test(
            "7e2c813d10f06d4b8af85389bc82eb18",
            "74fc41829b88f55e62aec4749685b323",
            "7a00c480b31d851359d78fad279dcd343bcd6a5f902ac0b55da656d735dbf329",
            130160,
            Variant::V2,
            "7e2c813d10f06d4b8af85389bc82eb18",
            "6ccb68ee6fc38a6e91f546f62b8e1a64b5223a4a0ef916e6062188c4ee15a879",
        );
    }
}
