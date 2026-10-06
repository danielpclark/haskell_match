# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path("../lib", __dir__)
require "minitest/autorun"
require "stringio"
require "haskell_match"

# Shared data types used across the suite.  Declared once; tests that need
# fresh types use unique names.
HaskellMatch.data "Maybe a = Nothing | Just a"
HaskellMatch.data "Either a b = Left a | Right b"
HaskellMatch.data "Shape = Circle Double | Rect Double Double | Tri Double Double Double"
HaskellMatch.data "Tree a = Leaf | Node (Tree a) a (Tree a)"
HaskellMatch.data "Person = Person { name :: String, age :: Int }"
HaskellMatch.data "Color = Red | Green | Blue"

module TestHelpers
  include Maybe
  include Either
  include Shape
  include Tree
  include Person
  include Color

  def fn(name = :f, **opts, &blk)
    HaskellMatch.fn(name, **opts, &blk)
  end

  def assert_raises_message(klass, pattern, &blk)
    err = assert_raises(klass, &blk)
    if pattern.is_a?(Regexp)
      assert_match pattern, err.message
    else
      assert_includes err.message, pattern
    end
    err
  end

  def capture_warnings
    old = $stderr
    $stderr = StringIO.new
    yield
    $stderr.string
  ensure
    $stderr = old
  end

  def node(left, value, right)
    Node.new(left, value, right)
  end
end
