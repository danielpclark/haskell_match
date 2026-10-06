# frozen_string_literal: true

require_relative "test_helper"

# Pattern syntax edge cases through the public API.
class SyntaxTest < Minitest::Test
  include TestHelpers

  def render(src)
    HaskellMatch::Native.render_pattern(src)
  end

  def test_whitespace_and_parentheses_are_insignificant
    assert_equal render("(x:xs)"), render("( x : xs )")
    assert_equal render("Just x"), render("(Just (x))")
    assert_equal render("(a, b)"), render("( a , b )")
  end

  def test_cons_is_right_associative
    assert_equal "(x:y:rest)", render("x:y:rest")
    assert_equal "(x:y:rest)", render("x:(y:rest)")
    assert_equal "((x:y):rest)", render("(x:y):rest")
  end

  def test_symbol_literals_versus_cons
    assert_equal "(x:xs)", render("x:xs")
    assert_equal "Just :ok", render("Just :ok")
    assert_equal "[:a, :b]", render("[:a,:b]")
    assert_equal "(:a:xs)", render("(:a : xs)")
  end

  def test_escapes
    assert_equal({ s: nil }.keys, HaskellMatch.pattern("s").names)
    f = fn { on("\"tab\\there\"") { :tab }; on("\"\\x41\\66\\u{1F600}\"") { :esc }; on("\"\\\"q\\\"\"") { :quoted }; on("_") { nil } }
    assert_equal :tab, f.("tab\there")
    assert_equal :esc, f.("AB\u{1F600}")
    assert_equal :quoted, f.('"q"')
  end

  def test_primes_and_underscores_in_names
    f = fn { on("(x', _y)") { |*vals| vals.first } }
    assert_equal 1, f.([1, 2])
    assert_equal ["x'", "_y"], f.bindings.first
  end

  def test_syntax_error_messages_point_at_the_problem
    err = assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("Just (x") { 1 } } }
    assert_match(/expected '\)'.*column \d+ in "Just \(x"/, err.message)
    err = assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("a b") { 1 } } }
    assert_includes err.message, "unexpected variable 'b' after pattern"
    err = assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("x $ y") { 1 } } }
    assert_includes err.message, "unexpected character '$'"
    err = assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("") { 1 } } }
    assert_includes err.message, "empty pattern"
    err = assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("'ab'") { 1 } } }
    assert_includes err.message, "exactly one character"
  end
end
