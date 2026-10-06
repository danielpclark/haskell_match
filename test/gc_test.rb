# frozen_string_literal: true

require_relative "test_helper"

class GcTest < Minitest::Test
  include TestHelpers

  def test_matching_under_gc_stress
    sum = fn(:sum) { on("[]") { 0 }; on("(x:xs)") { |x, xs| x + sum.(xs) } }
    walk = fn(:walk) do
      on("Leaf") { [] }
      on("Node l v r") { |l, v, r| walk.(l) + [v] + walk.(r) }
    end
    tree = Node.new(Node.new(Leaf, "a" * 20, Leaf), "b" * 20, Node.new(Leaf, "c" * 20, Leaf))
    GC.stress = true
    begin
      20.times do
        assert_equal 45, sum.((1..9).map { |i| i.to_s.to_i })
        assert_equal ["a" * 20, "b" * 20, "c" * 20], walk.(tree)
        assert_equal({ x: "q" * 30 }, HaskellMatch.pattern("Just x").match(Just.new("q" * 30)))
        assert_equal({ c: "q", cs: "q" * 29 }, HaskellMatch.pattern("(c:cs)").match("q" * 30))
      end
    ensure
      GC.stress = false
    end
  end

  def test_many_functions_and_patterns_survive_gc
    fns = (0...200).map do |i|
      fn("f#{i}") { on("Just x") { |x| x + i }; on("Nothing") { i } }
    end
    GC.start(full_mark: true, immediate_sweep: true)
    GC.compact if GC.respond_to?(:compact)
    fns.each_with_index { |f, i| assert_equal i + 1, f.(Just.new(1)) }
    pats = (0...100).map { |i| HaskellMatch.pattern("Just #{i}") }
    GC.start
    pats.each_with_index { |p, i| assert_equal({}, p.match(Just.new(i))) }
  end

  def test_literal_objects_are_kept_alive
    f = fn { on("('k', 1234567890123456789012345, 2.5, :sym, \"é\")") { :all }; on("_") { :other } }
    GC.start(full_mark: true, immediate_sweep: true)
    GC.compact if GC.respond_to?(:compact)
    assert_equal :all, f.(["k", 1_234_567_890_123_456_789_012_345, 2.5, :sym, "é"])
    assert_equal :other, f.(["z", 1, 2.5, :sym, "é"])
  end

  def test_bound_tails_survive_allocation_in_body
    rest = fn(:rest) { on("(_:xs)") { |xs| Array.new(50) { "pad" }; xs }; on("[]") { [] } }
    GC.stress = true
    begin
      5.times { assert_equal [2, 3], rest.([1, 2, 3]) }
    ensure
      GC.stress = false
    end
  end
end
