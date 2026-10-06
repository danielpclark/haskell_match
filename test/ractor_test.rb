# frozen_string_literal: true

require_relative "test_helper"

class RactorTest < Minitest::Test
  include TestHelpers

  def setup
    skip "Ractor not available" unless defined?(Ractor)
  end

  def quietly
    old = Warning[:experimental]
    Warning[:experimental] = false
    yield
  ensure
    Warning[:experimental] = old
  end

  def test_shareable_function_runs_in_another_ractor
    from_maybe = fn(:from_maybe, ractor: true) do
      on("d", "Nothing") { |d| d }
      on("_", "Just x", guard: ->(x) { x > 100 }) { :big }
      on("_", "Just x") { |x| x }
    end
    assert Ractor.shareable?(from_maybe)
    result = quietly do
      Ractor.new(from_maybe, Just.new(5)) { |f, v| [f.(0, v), f.(7, Nothing), f.(0, Just.new(500))] }.take
    end
    assert_equal [5, 7, :big], result
    # recursion through the function's own name or `recur` (a local variable
    # of the same name would shadow the name and is snapshotted as nil by
    # make_shareable, so the function is assigned to a differently named one)
    f = fn(:len, ractor: true) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + len.(xs) } }
    assert_equal 3, quietly { Ractor.new(f) { |g| g.([1, 2, 3]) }.take }
    fact = fn(ractor: true) { on("0") { 1 }; on("n") { |n| n * recur.(n - 1) } }
    assert_equal 120, quietly { Ractor.new(fact) { |f| f.(5) }.take }
    # tail calls in a Ractor
    go = fn(:go, ractor: true) { on("acc", "[]") { |acc| acc }; on("acc", "(x:xs)") { |acc, x, xs| recur.tail(acc + x, xs) } }
    assert_equal 5_000_050_000, quietly { Ractor.new(go) { |g| g.(0, (1..100_000).to_a) }.take }
  end

  def test_functions_with_a_non_shareable_self_are_rejected_only_when_asked
    plain = fn { on("x") { |x| x } }
    refute Ractor.shareable?(plain)
    err = assert_raises(Ractor::IsolationError) { Ractor.make_shareable(plain) }
    assert_includes err.message, "self is not shareable"
    f = fn(ractor: true) { on("x") { |x| x } }
    assert Ractor.shareable?(f)
    # a body capturing a non-shareable local cannot be made shareable
    unsafe_capture = "mutable".dup
    err = assert_raises(Ractor::IsolationError) { fn(ractor: true) { on("x") { |x| unsafe_capture + x } } }
    assert_includes err.message, "unsafe_capture"
    # ...but a shareable one is fine
    shareable_capture = "frozen"
    g = fn(ractor: true) { on("x") { |x| shareable_capture + x } }
    assert_equal "frozen!", quietly { Ractor.new(g) { |h| h.("!") }.take }
  end

  def test_data_values_are_shareable
    assert Ractor.shareable?(Just.new(1))
    assert Ractor.shareable?(Nothing)
    assert Ractor.shareable?(Person.new("a", 1))
  end

  def test_patterns_and_errors_in_another_ractor
    pat = Ractor.make_shareable(HaskellMatch.pattern("Just (x:_)"))
    result = quietly do
      Ractor.new(pat) do |p|
        [p.match(Just.new([1, 2])), p === Nothing, (p.match(5) rescue $!.class.name)]
      end.take
    end
    assert_equal [{ x: 1 }, false, "HaskellMatch::TypeMismatchError"], result
  end
end
