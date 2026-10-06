# frozen_string_literal: true

require_relative "test_helper"

class LazyListTest < Minitest::Test
  include TestHelpers

  def take
    @take ||= fn(:take) do
      on("0", "_") { [] }
      on("_", "[]") { [] }
      on("n", "(x:xs)") { |n, x, xs| [x] + take.(n - 1, xs) }
    end
  end

  def test_infinite_lists_match_list_patterns
    naturals = HaskellMatch.lazy(1..)
    assert_equal [1, 2, 3, 4, 5], take.(5, naturals)
    assert_equal [1, 2, 3, 4, 5], take.(5, naturals) # elements are memoised and reusable
    powers = HaskellMatch::LazyList.iterate(1) { |x| x * 2 }
    assert_equal [1, 2, 4, 8], take.(4, powers)
    assert_equal [1, 2, 4, 8], powers.take(4)
    assert_equal 1, powers.head
    assert_equal 2, powers.tail.head
    assert_equal "LazyList[1, 2, 4, 8, ...]", powers.inspect
  end

  def test_enumerators_are_wrapped_automatically
    assert_equal [1, 2, 3], take.(3, (1..).lazy)
    assert_equal [1, 2, 3], take.(3, (1..).lazy) # a wrap iterates from the start
    assert_equal [2, 4, 6], take.(3, (1..).lazy.map { |x| x * 2 })
    assert_equal [1, 2], take.(5, [1, 2].each)
    e = Enumerator.produce(1) { |x| x + 1 }
    assert_equal [1, 2], take.(2, e)
    assert_equal 1, e.next # the caller's enumerator was not consumed
  end

  def test_finite_lazy_lists_and_empty
    assert_equal [], take.(3, HaskellMatch.lazy([]))
    assert_equal [:a, :b], take.(9, HaskellMatch.lazy(%i[a b]))
    assert HaskellMatch::LazyList.empty.empty?
    assert_equal "LazyList[]", HaskellMatch::LazyList.empty.tap(&:force).inspect
    assert_raises(HaskellMatch::MatchError) { HaskellMatch::LazyList.empty.head }
    assert_equal [1, 2, 3], HaskellMatch.lazy(1..3).to_a
    assert_equal [1, 2, 3], HaskellMatch.lazy(1..3).each.to_a
    assert_equal 6, HaskellMatch.lazy(1..3).sum
    assert_equal [], take.(3, HaskellMatch::LazyList.generate { |y| })
    assert_equal [:z, :z], take.(2, HaskellMatch::LazyList.repeat(:z))
    assert_equal [5, 6], take.(2, HaskellMatch::LazyList.range(5))
    assert_equal [5], take.(2, HaskellMatch::LazyList.range(5, 5))
  end

  def test_exact_and_nested_patterns_on_lazy_lists
    f = fn(:f) do
      on("[a, b]") { |a, b| [:two, a, b] }
      on("[]") { :empty }
      on("(_:_)") { :other }
    end
    assert_equal [:two, 1, 2], f.(HaskellMatch.lazy([1, 2]))
    assert_equal :other, f.(HaskellMatch.lazy(1..))
    assert_equal :empty, f.(HaskellMatch.lazy([]))
    g = fn(:g) { on("Just (x:_)") { |x| x }; on("Just []") { :empty }; on("Nothing") { nil } }
    assert_equal 1, g.(Just[HaskellMatch.lazy(1..)])
    assert_equal :empty, g.(Just[HaskellMatch.lazy([])])
    assert_equal({ x: 1 }, HaskellMatch.pattern("(x:_)").match(HaskellMatch.lazy(1..)))
  end

  def test_haskell_style_stream_functions
    # zipWith (+) xs (tail xs) on an infinite stream, taking a prefix
    nats = HaskellMatch.lazy(0..)
    pairs = fn(:pairs) do
      on("0", "_") { [] }
      on("n", "(x:xs@(y:_))") { |n, x, xs, y| [x + y] + pairs.(n - 1, xs) }
      on("_", "_") { [] }
    end
    assert_equal [1, 3, 5, 7], pairs.(4, nats)
    # filtering a lazy list lazily with an Enumerator
    evens = HaskellMatch.lazy((0..).lazy.select(&:even?))
    assert_equal [0, 2, 4], take.(3, evens)
    sum_take = fn(:sum_take) do
      on("0", "_", "acc") { |acc| acc }
      on("n", "(x:xs)", "acc") { |n, x, xs, acc| sum_take.tail(n - 1, xs, acc + x) }
      on("_", "[]", "acc") { |acc| acc }
    end
    assert_equal 50_005_000, sum_take.(10_000, HaskellMatch.lazy(1..), 0)
  end

  def test_errors_while_forcing_propagate
    boom = Class.new(StandardError)
    bad = HaskellMatch.lazy(Enumerator.new { |y| y << 1; raise boom, "source failed" })
    assert_equal [1], take.(1, bad)
    assert_raises(boom) { take.(2, bad) }
    assert_raises(TypeError) { HaskellMatch.lazy(5) }
  end

  def test_type_mismatch_still_reported
    f = fn { on("[]") { 0 }; on("(_:_)") { 1 } }
    assert_raises(HaskellMatch::TypeMismatchError) { f.(5) }
  end
end
