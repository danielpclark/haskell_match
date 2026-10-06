# frozen_string_literal: true

require_relative "test_helper"

# `deep: true`: body invocation from Ruby code, for cheap deep recursion.
class DeepModeTest < Minitest::Test
  include TestHelpers

  def test_same_results_as_native_mode
    f = fn(:f, deep: true) do
      on("d", "Nothing") { |d| d }
      on(_, Just(x), guard: ->(x) { x > 100 }) { :big }
      on("_", "Just x") { |x| x }
    end
    assert f.deep?
    refute fn { on("x") { |x| x } }.deep?
    assert_equal 5, f.(0, Just[5])
    assert_equal 0, f.(0, Nothing)
    assert_equal :big, f.(0, Just[500])
    assert_equal 5, f[0, Just[5]]
    assert_equal 2, f.to_proc.arity
    assert_equal [1, 0], [Just[1], Nothing].map { |m| f.curry[0][m] }
    assert_raises(HaskellMatch::TypeMismatchError) { f.(0, 5) }
    assert_raises(ArgumentError) { f.(0) }
    err = assert_raises(ZeroDivisionError) { fn(deep: true) { on("x") { |x| 1 / x } }.(0) }
    assert_includes err.message, "divided by 0"
    r = catch(:done) { fn(deep: true) { on("x") { |x| throw :done, x } }.(7) }
    assert_equal 7, r
  end

  def test_deep_recursion_tail_and_defer
    count = fn(:count, deep: true) { on([]) { 0 }; on([_, *xs]) { |xs| 1 + count.(xs) } }
    assert_equal 200_000, count.((1..200_000).to_a)
    walk = fn(:walk, deep: true) { on(Leaf) { [] }; on(Node(l, v, r)) { |l, v, r| walk.(l) + [v] + walk.(r) } }
    deep = (1..50_000).inject(Leaf) { |t, i| Node.new(t, i, Leaf) }
    assert_equal (1..50_000).to_a, walk.(deep)
    go = fn(:go, deep: true) { on(acc, []) { |acc| acc }; on(acc, [x, *xs]) { |acc, x, xs| go.tail(acc + x, xs) } }
    assert_equal 500_000 * 500_001 / 2, go.(0, (1..500_000).to_a)
    even = odd = nil
    even = fn(:even, deep: true) { on(0) { true }; on(n) { |n| odd.tail(n - 1) } }
    odd = fn(:odd) { on(0) { false }; on(n) { |n| even.tail(n - 1) } } # mixed modes
    assert even.(100_001 - 1)
    length = fn(:length, deep: true) { on([]) { 0 }; on([_, *xs]) { |xs| length.defer(xs) { |n| 1 + n } } }
    assert_equal 300_000, length.((1..300_000).to_a)
    sum_tree = fn(:sum_tree, deep: true) do
      on(Leaf) { 0 }
      on(Node(l, v, r)) { |l, v, r| sum_tree.defer(l) { |ls| sum_tree.defer(r) { |rs| ls + v + rs } } }
    end
    assert_equal 50_000 * 50_001 / 2, sum_tree.(deep)
  end

  def test_depth_guard_and_segments_apply
    forever = fn(:forever, deep: true) { on(n) { |n| forever.(n + 1) + 1 } }
    HaskellMatch.max_depth = 3_000
    assert_raises(HaskellMatch::StackOverflowError) { forever.(0) }
    count = fn(:count, deep: true) { on([]) { 0 }; on([_, *xs]) { |xs| 1 + count.(xs) } }
    assert_equal 2_000, count.((1..2_000).to_a) # depth accounting intact after the error
    HaskellMatch.max_depth = 0
    HaskellMatch.stack_segment = 0
    assert_raises(SystemStackError) { count.((1..100_000).to_a) }
    HaskellMatch.stack_segment = 50
    assert_equal 100_000, count.((1..100_000).to_a)
  ensure
    HaskellMatch.max_depth = 250_000
    HaskellMatch.stack_segment = 100
  end

  def test_deep_by_default_switch
    HaskellMatch.deep_by_default = true
    assert fn { on("x") { |x| x } }.deep?
    refute fn(deep: false) { on("x") { |x| x } }.deep?
  ensure
    HaskellMatch.deep_by_default = false
  end

  # Memory per level is measured in fresh processes: within one process the
  # fiber pool keeps stacks from earlier deep calls, which hides the cost.
  def test_deep_mode_uses_less_memory_per_level_than_native
    skip "needs /proc" unless File.exist?("/proc/self/status")
    measure = lambda do |deep|
      script = <<~RUBY
        require "haskell_match"
        f = HaskellMatch.fn(:f, deep: #{deep}) { on([]) { 0 }; on([_, *xs]) { |xs| 1 + f.(xs) } }
        rss = -> { File.read("/proc/self/status")[/VmRSS:\\s+(\\d+)/, 1].to_i }
        list = (1..100_000).to_a
        GC.start
        base = rss.()
        f.(list)
        puts rss.() - base
      RUBY
      out = IO.popen([RbConfig.ruby, "-W0", "-I", File.expand_path("../lib", __dir__), "-e", script], &:read)
      Integer(out.strip)
    end
    native_kb = measure.(false)
    deep_kb = measure.(true)
    assert_operator deep_kb * 3, :<, native_kb, "deep #{deep_kb} KB vs native #{native_kb} KB for 100k levels"
  end
end
