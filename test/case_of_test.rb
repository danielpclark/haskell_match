# frozen_string_literal: true

require_relative "test_helper"

class CaseOfTest < Minitest::Test
  include TestHelpers

  def describe(v)
    HaskellMatch.case_of(v) do
      on("Just x", guard: ->(x) { x > 10 }) { |x| "big #{x}" }
      on("Just x") { |x| "just #{x}" }
      on("Nothing") { "nothing" }
    end
  end

  def test_case_expression
    assert_equal "big 11", describe(Just.new(11))
    assert_equal "just 1", describe(Just.new(1))
    assert_equal "nothing", describe(Nothing)
    assert_raises(HaskellMatch::TypeMismatchError) { describe(5) }
  end

  def test_multiple_scrutinees
    r = HaskellMatch.case_of([1, 2], true) do
      on("(x:_)", "True") { |x| x }
      on("_", "_") { 0 }
    end
    assert_equal 1, r
  end

  def test_case_is_cached
    HaskellMatch::CaseOf.clear_cache
    3.times { describe(Just.new(1)) }
    assert_equal 1, HaskellMatch::CaseOf::CACHE.size
    HaskellMatch.case_of(Nothing) { on("Nothing") { 1 }; on("Just _") { 2 } }
    assert_equal 2, HaskellMatch::CaseOf::CACHE.size
  end

  def test_case_enforces_exhaustiveness
    err = assert_raises(HaskellMatch::NonExhaustiveError) { HaskellMatch.case_of(Nothing) { on("Just x") { 1 } } }
    assert_includes err.message, "In a case expression for 'case expression at"
    assert_raises(HaskellMatch::RedundantClauseError) { HaskellMatch.case_of(Nothing) { on("_") { 1 }; on("Nothing") { 2 } } }
    assert_equal 1, HaskellMatch.case_of(Just.new(1), exhaustive: false) { on("Just x") { |x| x } }
    assert_raises(HaskellMatch::MatchError) { HaskellMatch.case_of(Nothing, exhaustive: false) { on("Just x") { |x| x } } }
  end

  def ten
    10
  end

  def test_bodies_close_over_locals_and_methods
    local = 5
    r = HaskellMatch.case_of(Just.new(1)) { on("Just x") { |x| x + local + ten }; on("Nothing") { 0 } }
    assert_equal 16, r
  end

  def test_instance_variables_need_the_explicit_builder_form
    @ivar = 10
    r = HaskellMatch.case_of(Just.new(1)) { |m| m.on("Just x") { |x| x + @ivar }; m.on("Nothing") { 0 } }
    assert_equal 11, r
    implicit = HaskellMatch.case_of(Just.new(1)) { on("Just x") { @ivar }; on("Nothing") { 0 } }
    assert_nil implicit
  end

  def test_errors
    assert_raises(ArgumentError) { HaskellMatch.case_of(1) }
    assert_raises(ArgumentError) { HaskellMatch.case_of { on("x") { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { HaskellMatch.case_of(1) { } }
  end

  def test_builder_argument_form
    r = HaskellMatch.case_of(Left.new(:e)) do |c|
      c.on("Left e") { |e| [:left, e] }
      c.on("Right v") { |v| [:right, v] }
    end
    assert_equal [:left, :e], r
  end
end
