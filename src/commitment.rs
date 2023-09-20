use plonky2::hash::hash_types::RichField;
use plonky2::iop::witness::{Witness, WitnessWrite};
use plonky2x::backend::circuit::PlonkParameters;
use plonky2x::frontend::ecc::ed25519::curve::curve_types::Curve;
use plonky2x::frontend::ecc::ed25519::curve::ed25519::Ed25519;

use plonky2x::frontend::merkle::tree::MerkleInclusionProofVariable;
use plonky2x::frontend::uint::uint64::U64Variable;
use plonky2x::frontend::vars::{ArrayVariable, Bytes32Variable, EvmVariable, U32Variable};
use plonky2x::prelude::{
    BoolVariable, ByteVariable, BytesVariable, CircuitBuilder, CircuitVariable, Variable,
};

use crate::consts::{
    HEADER_PROOF_DEPTH, PROTOBUF_BLOCK_ID_SIZE_BYTES, PROTOBUF_HASH_SIZE_BYTES, VARINT_SIZE_BYTES,
};
use crate::shared::TendermintHeader;

#[derive(Clone, Debug, CircuitVariable)]
#[value_name(CelestiaDataCommitmentProofInput)]
pub struct CelestiaDataCommitmentProofInputVariable<const WINDOW_SIZE: usize> {
    pub data_hashes: ArrayVariable<Bytes32Variable, WINDOW_SIZE>,
    pub block_heights: ArrayVariable<U64Variable, WINDOW_SIZE>,
    pub data_commitment_root: Bytes32Variable,
}

#[derive(Clone, Debug, CircuitVariable)]
#[value_name(HeaderVariableInput)]
pub struct HeaderVariable {
    pub header: Bytes32Variable,
    pub header_height_proof: MerkleInclusionProofVariable<HEADER_PROOF_DEPTH, VARINT_SIZE_BYTES>,
    pub height_byte_length: U32Variable,
    pub height: U64Variable,
}

#[derive(Clone, Debug, CircuitVariable)]
#[value_name(CelestiaHeaderChainProofInput)]
pub struct CelestiaHeaderChainProofInputVariable<const WINDOW_RANGE: usize> {
    pub current_header: HeaderVariable,
    pub trusted_header: HeaderVariable,
    pub data_hash_proofs: ArrayVariable<
        MerkleInclusionProofVariable<HEADER_PROOF_DEPTH, PROTOBUF_HASH_SIZE_BYTES>,
        WINDOW_RANGE,
    >,
    pub prev_header_proofs: ArrayVariable<
        MerkleInclusionProofVariable<HEADER_PROOF_DEPTH, PROTOBUF_BLOCK_ID_SIZE_BYTES>,
        WINDOW_RANGE,
    >,
}

pub trait CelestiaCommitment<L: PlonkParameters<D>, const D: usize> {
    type Curve: Curve;

    /// Encodes the marshalled varint into a BytesVariable<11>.
    /// Prepends a 0x00 byte for the leaf prefix and a 0x08 byte to the marshalled varint.
    fn encode_marshalled_varint(
        &mut self,
        marshalled_varint: &BytesVariable<9>,
    ) -> BytesVariable<11>;

    /// Encodes the data hash and height into a tuple.
    /// Spec: https://github.com/celestiaorg/celestia-core/blob/6933af1ead0ddf4a8c7516690e3674c6cdfa7bd8/rpc/core/blocks.go#L325-L334
    fn encode_data_root_tuple(
        &mut self,
        data_hash: &Bytes32Variable,
        height: &U64Variable,
    ) -> BytesVariable<64>;

    fn extract_hash_from_protobuf<const START_BYTE: usize, const PROTOBUF_LENGTH_BYTES: usize>(
        &mut self,
        bytes: &BytesVariable<PROTOBUF_LENGTH_BYTES>,
    ) -> Bytes32Variable;

    /// Compute the data commitment from the data hashes and block heights. WINDOW_RANGE is the number of blocks in the data commitment. NUM_LEAVES is the number of leaves in the tree for the data commitment.
    /// Assumes the data hashes are already proven.
    fn get_data_commitment<const WINDOW_RANGE: usize, const NB_LEAVES: usize>(
        &mut self,
        data_hashes: &ArrayVariable<Bytes32Variable, WINDOW_RANGE>,
        start_block: U64Variable,
    ) -> Bytes32Variable;

    /// Prove header chain from current_header to trusted_header & the block heights for the current header and the trusted header.
    /// Merkle prove the last block id against the current header, and the data hash for each header except the current header.
    /// Note: data_hash_proofs and prev_header_proofs should be in order from current_header to trusted_header
    fn prove_header_chain<const WINDOW_RANGE: usize>(
        &mut self,
        input: CelestiaHeaderChainProofInputVariable<WINDOW_RANGE>,
    );

    /// Prove the header chain from current_header to trusted_header & compute the data commitment.
    fn prove_data_commitment<const WINDOW_RANGE: usize, const NB_LEAVES: usize>(
        &mut self,
        input: CelestiaHeaderChainProofInputVariable<WINDOW_RANGE>,
        data_hashes: &ArrayVariable<Bytes32Variable, WINDOW_RANGE>,
    ) -> Bytes32Variable;
}

impl<L: PlonkParameters<D>, const D: usize> CelestiaCommitment<L, D> for CircuitBuilder<L, D> {
    type Curve = Ed25519;

    fn encode_marshalled_varint(
        &mut self,
        marshalled_varint: &BytesVariable<9>,
    ) -> BytesVariable<11> {
        // Prepend the 0x08 byte to the marshalled varint.
        let mut encoded_marshalled_varint = Vec::new();
        encoded_marshalled_varint.push(self.constant::<ByteVariable>(0u8));
        encoded_marshalled_varint.push(self.constant::<ByteVariable>(8u8));
        encoded_marshalled_varint.extend_from_slice(&marshalled_varint.0);
        BytesVariable(encoded_marshalled_varint.try_into().unwrap())
    }

    fn encode_data_root_tuple(
        &mut self,
        data_hash: &Bytes32Variable,
        height: &U64Variable,
    ) -> BytesVariable<64> {
        let mut encoded_tuple = Vec::new();

        // Encode the height.
        let encoded_height = height.encode(self);

        // Pad the abi.encodePacked(height) to 32 bytes. Height is 8 bytes, pad with 32 - 8 = 24 bytes.
        encoded_tuple.extend(
            self.constant::<ArrayVariable<ByteVariable, 24>>(vec![0u8; 24])
                .as_vec(),
        );

        // Add the abi.encodePacked(height) to the tuple.
        encoded_tuple.extend(encoded_height);

        // Add the data hash to the tuple.
        encoded_tuple.extend(data_hash.as_bytes().to_vec());

        // Convert Vec<ByteVariable> to BytesVariable<64>.
        BytesVariable::<64>(encoded_tuple.try_into().unwrap())
    }

    fn extract_hash_from_protobuf<const START_BYTE: usize, const PROTOBUF_LENGTH_BYTES: usize>(
        &mut self,
        bytes: &BytesVariable<PROTOBUF_LENGTH_BYTES>,
    ) -> Bytes32Variable {
        let vec_slice = bytes.0[START_BYTE..START_BYTE + 32].to_vec();
        let arr = ArrayVariable::<ByteVariable, 32>::new(vec_slice);
        Bytes32Variable(BytesVariable(arr.as_slice().try_into().unwrap()))
    }

    fn get_data_commitment<const WINDOW_RANGE: usize, const NB_LEAVES: usize>(
        &mut self,
        data_hashes: &ArrayVariable<Bytes32Variable, WINDOW_RANGE>,
        start_block: U64Variable,
    ) -> Bytes32Variable {
        let mut leaves = Vec::new();

        for i in 0..WINDOW_RANGE {
            let curr_idx = self.constant::<U64Variable>(i.into());
            let block_height = self.add(start_block, curr_idx);
            // Encode the data hash and height into a tuple.
            leaves.push(self.encode_data_root_tuple(&data_hashes[i], &block_height));
        }

        leaves.resize(NB_LEAVES, self.constant::<BytesVariable<64>>([0u8; 64]));

        let mut leaves_enabled = Vec::new();
        leaves_enabled.resize(WINDOW_RANGE, self.constant::<BoolVariable>(true));
        leaves_enabled.resize(NB_LEAVES, self.constant::<BoolVariable>(false));

        let root = self.compute_root_from_leaves::<NB_LEAVES, 64>(leaves, leaves_enabled);

        // Return the root hash.
        root
    }

    fn prove_header_chain<const WINDOW_RANGE: usize>(
        &mut self,
        input: CelestiaHeaderChainProofInputVariable<WINDOW_RANGE>,
    ) {
        // Verify current_block_height - trusted_block_height == WINDOW_RANGE
        let height_diff = self.sub(input.current_header.height, input.trusted_header.height);
        let window_range_target = self.constant::<U64Variable>(WINDOW_RANGE.into());
        self.assert_is_equal(height_diff, window_range_target);

        // Verify the current block's height
        self.verify_block_height(
            input.current_header.header,
            &input.current_header.header_height_proof.aunts,
            &input.current_header.height,
            input.current_header.height_byte_length,
        );

        // Verify the trusted block's height
        self.verify_block_height(
            input.trusted_header.header,
            &input.trusted_header.header_height_proof.aunts,
            &input.trusted_header.height,
            input.trusted_header.height_byte_length,
        );

        // Verify the header chain.
        let mut curr_header_hash = input.current_header.header;

        for i in 0..WINDOW_RANGE {
            let data_hash_proof = &input.data_hash_proofs[i];
            let prev_header_proof = &input.prev_header_proofs[i];

            let data_hash_proof_root = self
                .get_root_from_merkle_proof::<HEADER_PROOF_DEPTH, PROTOBUF_HASH_SIZE_BYTES>(
                    &data_hash_proof,
                );
            let prev_header_proof_root = self
                .get_root_from_merkle_proof::<HEADER_PROOF_DEPTH, PROTOBUF_BLOCK_ID_SIZE_BYTES>(
                    &prev_header_proof,
                );

            // Verify the prev header proof against the current header hash.
            self.assert_is_equal(prev_header_proof_root, curr_header_hash);

            // Extract the prev header hash from the prev header proof.
            let prev_header_hash = self
                .extract_hash_from_protobuf::<2, PROTOBUF_BLOCK_ID_SIZE_BYTES>(
                    &prev_header_proof.leaf,
                );

            // Verify the data hash proof against the prev header hash.
            self.assert_is_equal(data_hash_proof_root, prev_header_hash);

            curr_header_hash = prev_header_hash;
        }
        // Verify the last header hash in the chain is the trusted header.
        self.assert_is_equal(curr_header_hash, input.trusted_header.header);
    }

    fn prove_data_commitment<const WINDOW_RANGE: usize, const NB_LEAVES: usize>(
        &mut self,
        input: CelestiaHeaderChainProofInputVariable<WINDOW_RANGE>,
        data_hashes: &ArrayVariable<Bytes32Variable, WINDOW_RANGE>,
    ) -> Bytes32Variable {
        // Compute the data commitment.
        let data_commitment = self.get_data_commitment::<WINDOW_RANGE, NB_LEAVES>(
            data_hashes,
            input.trusted_header.height,
        );
        // Verify the header chain.
        self.prove_header_chain::<WINDOW_RANGE>(input);

        // Return the data commitment.
        data_commitment
    }
}

// To run tests with logs (i.e. to see proof generation time), set the environment variable `RUST_LOG=debug` before the test command.
// Alternatively, add env::set_var("RUST_LOG", "debug") to the top of the test.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use plonky2x::backend::circuit::DefaultParameters;

    use crate::{
        commitment::CelestiaCommitment,
        inputs::{generate_data_commitment_inputs, generate_header_chain_inputs},
    };

    type L = DefaultParameters;
    type F = <L as PlonkParameters<D>>::Field;
    const D: usize = 2;

    #[test]
    fn test_prove_data_commitment() {
        env_logger::try_init().unwrap_or_default();

        let mut builder = CircuitBuilder::<L, D>::new();

        const WINDOW_SIZE: usize = 4;
        const NUM_LEAVES: usize = 4;
        const START_BLOCK: usize = 3800;
        const END_BLOCK: usize = START_BLOCK + WINDOW_SIZE;

        let celestia_data_commitment_var =
            builder.read::<CelestiaDataCommitmentProofInputVariable<WINDOW_SIZE>>();

        let celestia_header_chain_var =
            builder.read::<CelestiaHeaderChainProofInputVariable<WINDOW_SIZE>>();

        let root_hash_target = builder.prove_data_commitment::<WINDOW_SIZE, NUM_LEAVES>(
            celestia_header_chain_var,
            &celestia_data_commitment_var.data_hashes,
        );
        builder.assert_is_equal(
            root_hash_target,
            celestia_data_commitment_var.data_commitment_root,
        );

        let circuit = builder.build();

        let mut input = circuit.input();
        input.write::<CelestiaDataCommitmentProofInputVariable<WINDOW_SIZE>>(
            generate_data_commitment_inputs::<WINDOW_SIZE, F>(START_BLOCK, END_BLOCK),
        );
        input.write::<CelestiaHeaderChainProofInputVariable<WINDOW_SIZE>>(
            generate_header_chain_inputs::<WINDOW_SIZE, F>(START_BLOCK, END_BLOCK),
        );
        let (proof, output) = circuit.prove(&input);
        circuit.verify(&proof, &input, &output);
    }

    #[test]
    fn test_data_commitment() {
        env_logger::try_init().unwrap_or_default();

        let mut builder = CircuitBuilder::<L, D>::new();

        const WINDOW_SIZE: usize = 4;
        const NUM_LEAVES: usize = 4;
        const START_BLOCK: usize = 3800;
        const END_BLOCK: usize = START_BLOCK + WINDOW_SIZE;

        let celestia_data_commitment_var =
            builder.read::<CelestiaDataCommitmentProofInputVariable<WINDOW_SIZE>>();

        let start_block = builder.constant::<U64Variable>(START_BLOCK.into());
        let root_hash_target = builder.get_data_commitment::<WINDOW_SIZE, NUM_LEAVES>(
            &celestia_data_commitment_var.data_hashes,
            start_block,
        );
        builder.assert_is_equal(
            root_hash_target,
            celestia_data_commitment_var.data_commitment_root,
        );

        let circuit = builder.build();

        let mut input = circuit.input();
        input.write::<CelestiaDataCommitmentProofInputVariable<WINDOW_SIZE>>(
            generate_data_commitment_inputs::<WINDOW_SIZE, F>(START_BLOCK, END_BLOCK),
        );
        let (proof, output) = circuit.prove(&input);
        circuit.verify(&proof, &input, &output);
    }

    #[test]
    fn test_prove_header_chain() {
        env_logger::try_init().unwrap_or_default();

        let mut builder = CircuitBuilder::<L, D>::new();

        const WINDOW_SIZE: usize = 4;
        const TRUSTED_BLOCK: usize = 3800;
        const CURRENT_BLOCK: usize = TRUSTED_BLOCK + WINDOW_SIZE;

        let celestia_header_chain_var =
            builder.read::<CelestiaHeaderChainProofInputVariable<WINDOW_SIZE>>();

        builder.prove_header_chain::<WINDOW_SIZE>(celestia_header_chain_var);

        let circuit = builder.build();

        let mut input = circuit.input();
        input.write::<CelestiaHeaderChainProofInputVariable<WINDOW_SIZE>>(
            generate_header_chain_inputs::<WINDOW_SIZE, F>(TRUSTED_BLOCK, CURRENT_BLOCK),
        );
        let (proof, output) = circuit.prove(&input);
        circuit.verify(&proof, &input, &output);
    }

    #[test]
    fn test_encode_varint() {
        env_logger::try_init().unwrap_or_default();

        let mut builder = CircuitBuilder::<L, D>::new();

        let height = builder.constant::<U64Variable>(3804.into());

        let encoded_height = builder.marshal_int64_varint(&height);
        let encoded_height = builder.encode_marshalled_varint(&BytesVariable(encoded_height));
        builder.watch(&encoded_height, "encoded_height");

        let circuit = builder.build();

        let input = circuit.input();
        let (proof, output) = circuit.prove(&input);
        circuit.verify(&proof, &input, &output);

        println!("Verified proof");
    }

    #[test]
    fn test_encode_data_root_tuple() {
        env_logger::try_init().unwrap_or_default();

        let mut builder = CircuitBuilder::<L, D>::new();

        let data_hash =
            builder.constant::<Bytes32Variable>(ethers::types::H256::from_slice(&[255u8; 32]));
        builder.watch(&data_hash, "data_hash");
        let height = builder.constant::<U64Variable>(256.into());
        builder.watch(&height, "height");
        let data_root_tuple = builder.encode_data_root_tuple(&data_hash, &height);
        builder.watch(&data_root_tuple, "data_root_tuple");
        builder.write(data_root_tuple);
        let circuit = builder.build();

        let mut expected_data_tuple_root = Vec::new();

        let expected_height = [
            0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1, 0,
        ];

        // Compute the expected output for testing
        let expected_data_root = vec![
            255u8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
            255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        ];

        expected_data_tuple_root.extend_from_slice(&expected_height);

        expected_data_tuple_root.extend_from_slice(&expected_data_root);

        let input = circuit.input();
        let (proof, mut output) = circuit.prove(&input);
        circuit.verify(&proof, &input, &output);

        let data_root_tuple_value = output.read::<ArrayVariable<ByteVariable, 64>>();
        assert_eq!(data_root_tuple_value, expected_data_tuple_root);

        println!("Verified proof");
    }
}