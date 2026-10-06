# frozen_string_literal: true

# Benchmarks haskell_match against hand-written Ruby and Ruby's built-in
# `case/in` pattern matching.  Run with `rake bench`.

require "benchmark"
require "haskell_match"

HaskellMatch.data "Maybe a = Nothing | Just a"
HaskellMatch.data "Shape = Circle Double | Rect Double Double | Tri Double Double Double"
include Maybe
include Shape

N = Integer(ENV.fetch("N", 1_000_000))

def report(label, iterations = N)
  t = Benchmark.realtime { yield }
  printf("  %-44s %8.1f ns/op\n", label, t / iterations * 1e9)
end

puts "haskell_match benchmarks (#{N} iterations each)"

# --- Maybe --------------------------------------------------------------
puts "\nfrom_maybe (2 arguments, constructor switch)"
from_maybe = HaskellMatch.fn(:from_maybe) do
  on("d", "Nothing") { |d| d }
  on("_", "Just x") { |x| x }
end
from_maybe_hm = HaskellMatch.fn(:from_maybe_kw) do
  on("d", "Nothing") { |d:| d }
  on("_", "Just x") { |x:| x }
end
NOTHING_CLASS = Nothing.class unless defined?(NOTHING_CLASS)
ruby_from_maybe = lambda do |d, m|
  case m
  when Just then m._1
  when NOTHING_CLASS then d
  else raise TypeError
  end
end
NOTHING_CLASS = Nothing.class
ruby_in_from_maybe = lambda do |d, m|
  case m
  in Just[x] then x
  in NOTHING_CLASS then d
  end
end
values = [Just.new(1), Nothing]
report("haskell_match fn") { i = 0; while i < N; from_maybe.(0, values[i & 1]); i += 1; end }
report("haskell_match fn (keyword bindings)") { i = 0; while i < N; from_maybe_hm.(0, values[i & 1]); i += 1; end }
report("ruby case/when + accessor") { i = 0; while i < N; ruby_from_maybe.(0, values[i & 1]); i += 1; end }
report("ruby case/in (Data deconstruct)") { i = 0; while i < N; ruby_in_from_maybe.(0, values[i & 1]); i += 1; end }

# --- lists --------------------------------------------------------------
puts "\nlist length (recursive cons patterns, 20 elements)"
len = HaskellMatch.fn(:len) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + len.(xs) } }
ruby_len = ->(xs) { xs.empty? ? 0 : 1 + ruby_len.(xs[1..]) }
ruby_in_len = lambda do |xs|
  case xs
  in [] then 0
  in [_, *rest] then 1 + ruby_in_len.(rest)
  end
end
list = (1..20).to_a
m = N / 20
report("haskell_match fn", m) { i = 0; while i < m; len.(list); i += 1; end }
report("ruby recursion with slices", m) { i = 0; while i < m; ruby_len.(list); i += 1; end }
report("ruby case/in with splat", m) { i = 0; while i < m; ruby_in_len.(list); i += 1; end }

# --- nested / literals ----------------------------------------------------
puts "\narea (3 constructors, nested arithmetic)"
area = HaskellMatch.fn(:area) do
  on("Circle r") { |r| 3.14159 * r * r }
  on("Rect w h") { |w, h| w * h }
  on("Tri a b c") { |a, b, c| s = (a + b + c) / 2.0; Math.sqrt(s * (s - a) * (s - b) * (s - c)) }
end
ruby_area = lambda do |s|
  case s
  in Circle[r] then 3.14159 * r * r
  in Rect[w, h] then w * h
  in Tri[a, b, c] then t = (a + b + c) / 2.0; Math.sqrt(t * (t - a) * (t - b) * (t - c))
  end
end
shapes = [Circle.new(1.0), Rect.new(2.0, 3.0), Tri.new(3.0, 4.0, 5.0)]
report("haskell_match fn") { i = 0; while i < N; area.(shapes[i % 3]); i += 1; end }
report("ruby case/in") { i = 0; while i < N; ruby_area.(shapes[i % 3]); i += 1; end }

puts "\nfib (integer literals + guards fallthrough), fib(20)"
fib = HaskellMatch.fn(:fib) { on("0") { 0 }; on("1") { 1 }; on("n") { |n| fib.(n - 1) + fib.(n - 2) } }
ruby_fib = ->(n) { n < 2 ? n : ruby_fib.(n - 1) + ruby_fib.(n - 2) }
m = [N / 20_000, 1].max
report("haskell_match fn", m) { m.times { fib.(20) } }
report("ruby if/else", m) { m.times { ruby_fib.(20) } }

puts "\ncase_of expression (compiled once, blocks collected per call)"
report("haskell_match case_of") do
  i = 0
  while i < N
    HaskellMatch.case_of(values[i & 1]) { on("Just x") { |x| x }; on("Nothing") { 0 } }
    i += 1
  end
end
pat = HaskellMatch.pattern("Just x")
report("haskell_match pattern#match") { i = 0; while i < N; pat.match(values[i & 1]); i += 1; end }
