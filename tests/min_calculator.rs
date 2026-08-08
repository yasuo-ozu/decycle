//! Run under BOTH algorithms (ranked + structural) via the shared `dual_mod!`/`on_both!` harness.
mod common;

dual_mod! {
    calculator {
        #[decycle]
        pub trait Evaluate {
            fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32;
        }

        pub struct Expr;
        pub struct Term;

        impl Evaluate for Expr
        where
            Term: Evaluate,
        {
            fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32 {
                let left_val = Term.evaluate(input, index);
                let op = input[*index];
                *index += 1;
                let right_val = Term.evaluate(input, index);
                match op {
                    "+" => left_val + right_val,
                    "-" => left_val - right_val,
                    _ => left_val,
                }
            }
        }

        impl Evaluate for Term
        where
            Expr: Evaluate,
        {
            fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32 {
                let token = input[*index];
                *index += 1;
                if token == "(" {
                    let result = Expr.evaluate(input, index);
                    *index += 1; // skip closing ')'
                    result
                } else {
                    token.parse::<i32>().unwrap()
                }
            }
        }
    }
}

#[test]
fn test_simple_addition() {
    on_both!(calculator, {
        let expr = Expr;
        let input = vec!["2", "+", "3"];
        let mut index = 0;
        assert_eq!(expr.evaluate(&input, &mut index), 5);
    });
}

#[test]
fn test_simple_subtraction() {
    on_both!(calculator, {
        let expr = Expr;
        let input = vec!["5", "-", "2"];
        let mut index = 0;
        assert_eq!(expr.evaluate(&input, &mut index), 3);
    });
}

#[test]
fn test_single_number() {
    on_both!(calculator, {
        let term = Term;
        let input = vec!["42"];
        let mut index = 0;
        assert_eq!(term.evaluate(&input, &mut index), 42);
    });
}

#[test]
fn test_parenthesized_simple() {
    on_both!(calculator, {
        let term = Term;
        let input = vec!["(", "1", "+", "2", ")"];
        let mut index = 0;
        assert_eq!(term.evaluate(&input, &mut index), 3);
    });
}

#[test]
fn test_nested_expression() {
    on_both!(calculator, {
        let expr = Expr;
        let input = vec!["1", "+", "(", "2", "-", "3", ")"];
        let mut index = 0;
        assert_eq!(expr.evaluate(&input, &mut index), 0); // 1 + (2 - 3) = 0
    });
}
