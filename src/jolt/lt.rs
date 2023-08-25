use std::marker::PhantomData;

use ark_ff::PrimeField;
use ark_std::log2;

use crate::utils::split_bits;

use super::{InstructionStrategy, JoltStrategy, SubtableStrategy};

pub enum LTVMInstruction {
  LT(u64, u64),
}

pub struct LTVM {}

impl<F: PrimeField> JoltStrategy<F> for LTVM {
  type Instruction = LTVMInstruction;

  fn instructions() -> Vec<Box<dyn InstructionStrategy<F>>> {
    vec![Box::new(LTInstruction {_marker: PhantomData::<F>})]
  }

  fn primary_poly_degree() -> usize {
    // LT[C-1] * EQ[0] * ... * EQ[C-2] + 1
    4
  }
}

pub struct LTInstruction<F: PrimeField> {
    _marker: PhantomData<F>
}

impl<F: PrimeField> InstructionStrategy<F> for LTInstruction<F> {
  fn subtables(&self) -> Vec<Box<dyn super::SubtableStrategy<F>>> {
    vec![
        Box::new(LTSubtable {_marker: PhantomData::<F>}), 
        Box::new(EQSubtable {_marker: PhantomData::<F>})
    ]
  }

  fn combine_lookups(&self, vals: &[F]) -> F {
    assert_eq!(vals.len(), self.num_memories());
    let mut sum = F::zero();
    let mut eq_prod = F::one();

    let C: usize = self.subtables()[0].dimensions();

    for i in 0..C {
      sum += vals[2 * i] * eq_prod;
      eq_prod *= vals[2 * i + 1];
    }
    sum
  }

  fn g_poly_degree(&self) -> usize {
    todo!()
  }
}

pub struct LTSubtable<F: PrimeField> {
    _marker: PhantomData<F>
}
impl<F: PrimeField> SubtableStrategy<F> for LTSubtable<F> {
  fn dimensions(&self) -> usize {
    8
  }

  fn memory_size(&self) -> usize {
    1 << 16
  }

  fn materialize(&self) -> Vec<F> {
    let M: usize = self.memory_size();
    let bits_per_operand = (log2(M) / 2) as usize;

    let mut materialized_lt: Vec<F> = Vec::with_capacity(M);

    // Materialize table in counting order where lhs | rhs counts 0->m
    for idx in 0..M {
      let (lhs, rhs) = split_bits(idx, bits_per_operand);
      materialized_lt.push(F::from((lhs < rhs) as u64));
    }

    materialized_lt
  }

  fn evaluate_mle(&self, point: &[F]) -> F {
    debug_assert!(point.len() % 2 == 0);
    let b = point.len() / 2;
    let (x, y) = point.split_at(b);

    let mut result = F::zero();
    let mut eq_term = F::one();
    for i in 0..b {
    result += (F::one() - x[i]) * y[i] * eq_term;
    eq_term *= F::one() - x[i] - y[i] + F::from(2u64) * x[i] * y[i];
    }
    result
  }
}

pub struct EQSubtable<F: PrimeField> {
    _marker: PhantomData<F>
}
impl<F: PrimeField> SubtableStrategy<F> for EQSubtable<F> {
  fn dimensions(&self) -> usize {
    8
  }

  fn memory_size(&self) -> usize {
    1 << 16
  }

  fn materialize(&self) -> Vec<F> {
    let M: usize = self.memory_size();
    let bits_per_operand = (log2(M) / 2) as usize;

    let mut materialized_eq: Vec<F> = Vec::with_capacity(M);

    // Materialize table in counting order where lhs | rhs counts 0->m
    for idx in 0..M {
      let (lhs, rhs) = split_bits(idx, bits_per_operand);
      materialized_eq.push(F::from((lhs == rhs) as u64));
    }

    materialized_eq
  }

  fn evaluate_mle(&self, point: &[F]) -> F {
    debug_assert!(point.len() % 2 == 0);
    let b = point.len() / 2;
    let (x, y) = point.split_at(b);

    let mut eq_term = F::one();
    for i in 0..b {
        eq_term *= F::one() - x[i] - y[i] + F::from(2u64) * x[i] * y[i];
    }
    eq_term
  }
}

#[cfg(test)]
mod tests {
  use ark_curve25519::{EdwardsProjective, Fr};
  use ark_ff::PrimeField;
  use ark_std::{log2, test_rng};
  use merlin::Transcript;
  use rand_chacha::rand_core::RngCore;

  use crate::{
    jolt::lt::LTVM,
    lasso::{
      densified::DensifiedRepresentation,
      surge::{SparsePolyCommitmentGens, SparsePolynomialEvaluationProof},
    },
    utils::random::RandomTape,
  };

  pub fn gen_indices<const C: usize>(sparsity: usize, memory_size: usize) -> Vec<Vec<usize>> {
    let mut rng = test_rng();
    let mut all_indices: Vec<Vec<usize>> = Vec::new();
    for _ in 0..sparsity {
      let indices = vec![rng.next_u64() as usize % memory_size; C];
      all_indices.push(indices);
    }
    all_indices
  }

  pub fn gen_random_point<F: PrimeField>(memory_bits: usize) -> Vec<F> {
    let mut rng = test_rng();
    let mut r_i: Vec<F> = Vec::with_capacity(memory_bits);
    for _ in 0..memory_bits {
      r_i.push(F::rand(&mut rng));
    }
    r_i
  }

  #[test]
  fn e2e() {
    const C: usize = 8;
    const S: usize = 1 << 8;
    const M: usize = 1 << 16;

    let log_m = log2(M) as usize;
    let log_s: usize = log2(S) as usize;

    let nz: Vec<Vec<usize>> = gen_indices::<C>(S, M);
    let r: Vec<Fr> = gen_random_point::<Fr>(log_s);

    let mut dense: DensifiedRepresentation<Fr, LTVM> =
      DensifiedRepresentation::from_lookup_indices(&nz, log_m);
    let gens =
      SparsePolyCommitmentGens::<EdwardsProjective>::new(b"gens_sparse_poly", C, S, C, log_m);
    let commitment = dense.commit::<EdwardsProjective>(&gens);
    let mut random_tape = RandomTape::new(b"proof");
    let mut prover_transcript = Transcript::new(b"example");
    let proof = SparsePolynomialEvaluationProof::<EdwardsProjective, LTVM>::prove(
      &mut dense,
      &r,
      &gens,
      &mut prover_transcript,
      &mut random_tape,
    );

    let mut verify_transcript = Transcript::new(b"example");
    proof
      .verify(&commitment, &r, &gens, &mut verify_transcript)
      .expect("should verify");
  }
}
