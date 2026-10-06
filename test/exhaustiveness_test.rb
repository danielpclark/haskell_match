# frozen_string_literal: true

require_relative "test_helper"

class ExhaustivenessTest < Minitest::Test
  include TestHelpers

  def missing_for(&blk)
    assert_raises(HaskellMatch::NonExhaustiveError, &blk).missing
  end

  def test_non_exhaustive_is_an_error_by_default
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn(:f) { on("Just x") { |x| x } } }
    assert_equal <<~MSG.chomp, err.message
      Pattern match(es) are non-exhaustive
      In an equation for 'f':
          Patterns not matched:
              Nothing
    MSG
    assert_equal ["Nothing"], err.missing
    assert_kind_of HaskellMatch::CompileError, err
  end

  def test_missing_pattern_witnesses
    assert_equal ["[]"], missing_for { fn { on("(x:xs)") { 1 } } }
    assert_equal ["(_:_:_)"], missing_for { fn { on("[]") { 1 }; on("[x]") { 2 } } }
    assert_equal ["[_]"], missing_for { fn { on("[]") { 1 }; on("(x:y:rest)") { 2 } } }
    assert_equal ["False"], missing_for { fn { on("True") { 1 } } }
    assert_equal ["Just (Just _)"], missing_for { fn { on("Nothing") { 1 }; on("Just Nothing") { 2 } } }
    assert_equal ["Rect _ _", "Tri _ _ _"], missing_for { fn { on("Circle _") { 1 } } }
    assert_equal ["Node Leaf _ _"], missing_for { fn { on("Leaf") { 1 }; on("Node (Node _ _ _) _ _") { 2 } } }
    assert_equal ["(True, Just _)"], missing_for { fn { on("(True, Nothing)") { 1 }; on("(False, _)") { 2 } } }
    assert_equal ["[] (_:_)", "(_:_) []"], missing_for { fn { on("(x:xs)", "(y:ys)") { 1 }; on("[]", "[]") { 2 } } }
    assert_equal ["(Circle _) (Rect _ _)", "(Circle _) (Tri _ _ _)", "(Rect _ _) _", "(Tri _ _ _) _"],
                 missing_for { fn { on("Circle _", "Circle _") { 1 } } }
    assert_equal ["Red", "Blue"], missing_for { fn { on("Green") { 1 } } }
    assert_equal ["Person _ p1 where p1 is not one of {0}"], missing_for { fn { on("Person { age = 0 }") { 1 } } }
  end

  def test_literal_witnesses
    assert_equal ["p1 where p1 is not one of {0, 1}"], missing_for { fn { on("0") { 1 }; on("1") { 2 } } }
    assert_equal ["p1 where p1 is not one of {'a', 'b'}"], missing_for { fn { on("'a'") { 1 }; on("'b'") { 2 } } }
    # a string literal is a list of characters
    assert_equal ["[]", "['o']", "('o':'k':_:_)", "('o':p1:_) where p1 is not one of {'k'}", "(p1:_) where p1 is not one of {'o'}"],
                 missing_for { fn { on("\"ok\"") { 1 }; on("(c:cs)", guard: ->(c) { c == "x" }) { 2 } } }
    assert_equal ["p1 where p1 is not one of {:ok}"], missing_for { fn { on(":ok") { 1 } } }
    assert_equal ["Just p1 where p1 is not one of {0}"], missing_for { fn { on("Just 0") { 1 }; on("Nothing") { 2 } } }
    assert_equal ["(p1, p2) where p1 is not one of {0} and p2 is not one of {1}"],
                 missing_for { fn { on("(0, _)") { 1 }; on("(_, 1)") { 2 } } }
    assert_equal ["Just (-1)"], [HaskellMatch::Native.render_pattern("Just (-1)")]
  end

  def test_exhaustive_definitions_are_accepted
    fn { on("Nothing") { 1 }; on("Just _") { 2 } }
    fn { on("[]") { 1 }; on("[_]") { 2 }; on("(_:_:_)") { 3 } }
    fn { on("0") { 1 }; on("n") { 2 } }
    fn { on("(a, b)") { 1 } }
    fn { on("Leaf") { 1 }; on("Node Leaf _ _") { 2 }; on("Node (Node _ _ _) _ _") { 3 } }
    fn { on("Red") { 1 }; on("Green") { 2 }; on("Blue") { 3 } }
    fn { on("Left _") { 1 }; on("Right _") { 2 } }
    fn { on("~(Just x)") { 1 } }
    fn { on("a@(Just _)") { 1 }; on("Nothing") { 2 } }
    fn { on("(x:xs)", "(y:ys)") { 1 }; on("_", "_") { 2 } }
    fn { on("Just x", guard: ->(x) { x > 0 }) { 1 }; on("Just _") { 2 }; on("Nothing") { 3 } }
    fn { on("x", guard: ->(x) { x }) { 1 }; on("_", guard: otherwise) { 2 } }
  end

  def test_guards_do_not_count_towards_coverage
    err = assert_raises(HaskellMatch::NonExhaustiveError) do
      fn(:g) { on("Just x", guard: ->(x) { x > 0 }) { 1 }; on("Nothing") { 2 } }
    end
    assert_equal ["Just _"], err.missing
    err = assert_raises(HaskellMatch::NonExhaustiveError) do
      fn(:g) { on("n", guard: ->(n) { n > 0 }) { 1 }; on("n", guard: ->(n) { n <= 0 }) { 2 } }
    end
    assert_equal ["_"], err.missing
    # but `otherwise` does
    fn { on("n", guard: ->(n) { n > 0 }) { 1 }; on("n", guard: otherwise) { 2 } }
  end

  def test_redundant_clauses_are_an_error_by_default
    err = assert_raises(HaskellMatch::RedundantClauseError) { fn(:f) { on("_") { 1 }; on("Just x") { 2 } } }
    assert_match(/\APattern match is redundant\nIn an equation for 'f':\n    f Just x = \.\.\. \(.*exhaustiveness_test\.rb:\d+\)\z/, err.message)
    assert_equal [1], err.clauses
    err = assert_raises(HaskellMatch::RedundantClauseError) do
      fn(:f) { on("[]") { 1 }; on("(x:xs)") { 2 }; on("[a, b]") { 3 }; on("_") { 4 } }
    end
    assert_equal [2, 3], err.clauses
    assert_includes err.message, "Pattern matches are redundant"
    assert_raises(HaskellMatch::RedundantClauseError) { fn { on("0") { 1 }; on("0") { 2 }; on("_") { 3 } } }
    assert_raises(HaskellMatch::RedundantClauseError) { fn { on("Just _") { 1 }; on("Just 0") { 2 }; on("Nothing") { 3 } } }
    assert_raises(HaskellMatch::RedundantClauseError) do
      fn { on("(True, _)") { 1 }; on("(_, True)") { 2 }; on("(False, False)") { 3 }; on("(True, True)") { 4 } }
    end
    # a guarded clause after a total one is redundant
    assert_raises(HaskellMatch::RedundantClauseError) do
      fn { on("Just _") { 1 }; on("Just x", guard: ->(x) { x }) { 2 }; on("Nothing") { 3 } }
    end
    # a guarded clause does not shadow a later identical clause
    fn { on("Just x", guard: ->(x) { x }) { 1 }; on("Just x") { 2 }; on("Nothing") { 3 } }
  end

  def test_policies
    out = capture_warnings { fn(:w, exhaustive: :warn) { on("Just x") { 1 } } }
    assert_includes out, "haskell_match: Pattern match(es) are non-exhaustive"
    assert_includes out, "Nothing"
    out = capture_warnings { fn(:w, exhaustive: :ignore) { on("Just x") { 1 } } }
    assert_equal "", out
    out = capture_warnings { fn(:w, exhaustive: false) { on("Just x") { 1 } } }
    assert_equal "", out
    out = capture_warnings { fn(:w, overlapping: :warn) { on("_") { 1 }; on("Just x") { 2 } } }
    assert_includes out, "redundant"
    out = capture_warnings { fn(:w, overlapping: :ignore) { on("_") { 1 }; on("Just x") { 2 } } }
    assert_equal "", out
    f = fn(:w, overlapping: false) { on("_") { 1 }; on("Just x") { 2 } }
    assert_equal 1, f.(Just.new(1))
    assert_raises(HaskellMatch::NonExhaustiveError) { fn(exhaustive: true) { on("Just x") { 1 } } }
    assert_raises(HaskellMatch::RedundantClauseError) { fn(overlapping: true) { on("_") { 1 }; on("_") { 2 } } }
  end

  def test_global_defaults
    assert_equal :error, HaskellMatch.exhaustive
    assert_equal :error, HaskellMatch.overlapping
    HaskellMatch.exhaustive = :ignore
    HaskellMatch.overlapping = :ignore
    f = fn { on("Just x") { 1 }; on("Just 0") { 2 } }
    assert_equal 1, f.(Just.new(0))
    assert_raises(HaskellMatch::MatchError) { f.(Nothing) }
  ensure
    HaskellMatch.exhaustive = nil
    HaskellMatch.overlapping = nil
  end

  def test_witness_list_is_truncated
    HaskellMatch.data "Big = Big0 | Big1 | Big2 | Big3 | Big4 | Big5"
    # 4 arguments give exactly 20 witnesses: reported in full, no ellipsis
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on("Big0", "Big0", "Big0", "Big0") { 1 } } }
    assert_equal 20, err.missing.size
    refute_includes err.message, "..."
    # 5 arguments give 25: cut at the limit and marked as truncated
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on("Big0", "Big0", "Big0", "Big0", "Big0") { 1 } } }
    assert_equal HaskellMatch::Compiler::WITNESS_LIMIT, err.missing.size
    assert_includes err.message, "..."
  end

  def test_redundancy_is_reported_before_exhaustiveness
    assert_raises(HaskellMatch::RedundantClauseError) { fn { on("Just x") { 1 }; on("Just 0") { 2 } } }
  end
end
