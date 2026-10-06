# frozen_string_literal: true

require_relative "test_helper"

# Direct exercise of the native boundary: bad arguments must raise, never
# crash.
class NativeTest < Minitest::Test
  include TestHelpers

  N = HaskellMatch::Native

  def test_parse_data
    assert_equal ["Maybe", ["a"], [["Nothing", 0, nil], ["Just", 1, nil]]], N.parse_data("Maybe a = Nothing | Just a")
    assert_equal [["P", 2, %w[a b]]], N.parse_data("P = P { a :: Int, b :: Int }")[2]
    assert_raises(HaskellMatch::DataDeclarationError) { N.parse_data("= X") }
    assert_raises(ArgumentError) { N.parse_data(5) }
  end

  def test_constructors
    assert_equal %w[Nothing Just], N.constructors("Maybe")
    assert_equal %w[False True], N.constructors("Bool")
    assert_nil N.constructors("Nope")
  end

  def test_register_type_argument_errors
    assert_raises(ArgumentError) { N.register_type(5, []) }
    assert_raises(ArgumentError) { N.register_type("T", 5) }
    assert_raises(ArgumentError) { N.register_type("T", [["A", 0, nil]]) }
    assert_raises(ArgumentError) { N.register_type("T", [["A", "x", nil, Object]]) }
    assert_raises(ArgumentError) { N.register_type("T", [["A", -1, nil, Object]]) }
    assert_raises(ArgumentError) { N.register_type("T", [["A", 0, nil, nil]]) }
    assert_raises(ArgumentError) { N.register_type("T", [["A", 1, "fields", Object]]) }
    assert_raises(HaskellMatch::DataDeclarationError) { N.register_type("T", []) }
    assert_raises(HaskellMatch::DataDeclarationError) { N.register_type("T", [["A", 1, ["a", "b"], Object]]) }
    refute N.constructors("T")
  end

  def test_matcher_argument_errors
    assert_raises(ArgumentError) { N::Matcher.new(1, [["x"]], [false], 20) }
    assert_raises(ArgumentError) { N::Matcher.new("f", "x", [false], 20) }
    assert_raises(ArgumentError) { N::Matcher.new("f", [], [], 20) }
    assert_raises(ArgumentError) { N::Matcher.new("f", [["x"]], [], 20) }
    assert_raises(HaskellMatch::PatternSyntaxError) { N::Matcher.new("f", ["x"], [false], 20) }
    assert_raises(HaskellMatch::PatternSyntaxError) { N::Matcher.new("f", [[1]], [false], 20) }
    assert_raises(HaskellMatch::ClauseArityError) { N::Matcher.new("f", [[]], [false], 20) }
    assert_raises(TypeError) { N::Matcher.allocate }
  end

  def test_matcher_runtime_argument_errors
    m = N::Matcher.new("f", [["Just x"], ["Nothing"]], [false, false], 20)
    assert_equal 1, m.arity
    assert_equal "f", m.name
    assert_equal [["x"], []], m.names
    assert_equal [false, false], m.guarded
    assert_operator m.slots, :>=, 2
    assert_raises(ArgumentError) { m.select_with(5, nil) }
    assert_raises(ArgumentError) { m.select_with([Nothing], 5) }
    assert_raises(ArgumentError) { m.run([Nothing], 5, nil) }
    assert_raises(ArgumentError) { m.run([Nothing], [nil, nil], nil) }
    assert_raises(ArgumentError) { m.run(5, [], nil) }
    assert_raises(ArgumentError) { m.select(Nothing, Nothing) }
    assert_raises(ArgumentError) { m.call(Nothing) } # no bodies attached
    assert_equal [1, []], m.select_with([Nothing], nil)
    assert_equal [1, []], m.select(Nothing)
    assert_equal [0, [3]], m.select_with([Just.new(3)], nil)
    assert_equal 4, m.run([Just.new(3)], [->(x) { x + 1 }, -> { 0 }], nil)
    # a guard array shorter than the clause list is treated as no guard
    g = N::Matcher.new("g", [["x"], ["_"]], [true, false], 20)
    assert_equal [0, [1]], g.select_with([1], [])
    assert_equal [1, []], g.select_with([1], [->(_) { false }])
    assert_equal [0, [1]], g.select_with([1], [nil])
    # select returns nil (not an error) when nothing matches
    p = N::Matcher.new("p", [["Just x"]], [false], 20)
    assert_nil p.select_with([Nothing], nil)
    assert_nil p.select(Nothing)
    assert_raises(HaskellMatch::MatchError) { p.run([Nothing], [->(x) { x }], nil) }
    # methods reject receivers that are not matchers
    assert_raises(TypeError) { N::Matcher.instance_method(:arity).bind_call(Object.new) }
  end

  def test_wide_clauses_use_the_array_path
    pats = (1..70).map { |i| "x#{i}" }
    m = N::Matcher.new("wide", [pats], [false], 20)
    args = (1..70).to_a
    assert_equal [0, args], m.select(*args)
    assert_equal 70, m.run(args, [->(*vals) { vals.size }], nil)
    g = N::Matcher.new("wide_guard", [pats, pats], [true, false], 20)
    assert_equal [1, args], g.select_with(args, [->(*vals) { vals.sum.zero? }])
  end

  def test_many_slots
    pats = (1..40).map { |i| "x#{i}" }
    m = N::Matcher.new("wide", [pats], [false], 20)
    assert_operator m.slots, :>=, 40
    assert_equal [0, (1..40).to_a], m.select(*(1..40).to_a)
  end

  def test_loader_reports_path
    assert File.file?(HaskellMatch::NativeLoader.path)
  end
end
