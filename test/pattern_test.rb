# frozen_string_literal: true

require_relative "test_helper"

class PatternTest < Minitest::Test
  include TestHelpers

  def test_match_returns_bindings_hash
    p = HaskellMatch.pattern("Just (x:rest)")
    assert_equal({ x: 1, rest: [2, 3] }, p.match(Just.new([1, 2, 3])))
    assert_nil p.match(Just.new([]))
    assert_nil p.match(Nothing)
    assert_equal %i[x rest], p.names
    assert_equal "Just (x:rest)", p.source
    assert_equal "#<HaskellMatch::Pattern Just (x:rest)>", p.inspect
    refute p.irrefutable?
    assert HaskellMatch.pattern("(a, _)").irrefutable?
    assert HaskellMatch.pattern("x").irrefutable?
  end

  def test_match_bang
    p = HaskellMatch["Just x"]
    assert_equal({ x: 1 }, p.match!(Just.new(1)))
    assert_raises(HaskellMatch::MatchError) { p.match!(Nothing) }
  end

  def test_type_mismatch_versus_case_equality
    p = HaskellMatch.pattern("Just x")
    assert_raises(HaskellMatch::TypeMismatchError) { p.match(5) }
    refute p === 5
    assert p === Just.new(1)
    refute p === Nothing
  end

  def test_case_when_integration
    r = case Just.new([7])
        when HaskellMatch["Nothing"] then :none
        when HaskellMatch["Just []"] then :empty
        when HaskellMatch["Just (_:_)"] then :some
        end
    assert_equal :some, r
  end

  def test_no_bindings
    assert_equal({}, HaskellMatch.pattern("Nothing").match(Nothing))
    assert_equal({}, HaskellMatch.pattern("[]").match([]))
    assert_equal({}, HaskellMatch.pattern("0").match(0))
    assert_nil HaskellMatch.pattern("0").match(1)
  end

  def test_to_proc
    p = HaskellMatch.pattern("Just x")
    assert_equal [{ x: 1 }, nil], [Just.new(1), Nothing].map(&p)
  end

  def test_errors
    assert_raises(ArgumentError) { HaskellMatch.pattern(5) }
    assert_raises(HaskellMatch::PatternSyntaxError) { HaskellMatch.pattern("") }
    assert_raises(HaskellMatch::PatternSyntaxError) { HaskellMatch.pattern("Just (") }
    assert_raises(HaskellMatch::UnknownConstructorError) { HaskellMatch.pattern("Nope") }
  end

  def test_render_pattern_canonical_form
    assert_equal "(x:y:rest)", HaskellMatch::Native.render_pattern("x : y : rest")
    assert_equal "[a, b]", HaskellMatch::Native.render_pattern("[ a , b ]")
    assert_equal "Just (Just _)", HaskellMatch::Native.render_pattern("Just (Just _)")
    assert_equal "Person _ a", HaskellMatch::Native.render_pattern("Person { age = a }")
    assert_equal "all@(x:_)", HaskellMatch::Native.render_pattern("all@(x:_)")
    assert_equal "~(Just y)", HaskellMatch::Native.render_pattern("~(Just y)")
    assert_equal "x", HaskellMatch::Native.render_pattern("!x")
    assert_equal "Just \"s\"", HaskellMatch::Native.render_pattern("Just 's'")
    assert_equal "2", HaskellMatch::Native.render_pattern("2.0")
  end
end
