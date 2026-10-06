# frozen_string_literal: true

# Haskell-style pattern matching for Ruby, compiled to decision trees in Rust.
#
#   require "haskell_match"
#
#   HaskellMatch.data "Maybe a = Nothing | Just a"
#   include Maybe
#
#   from_maybe = HaskellMatch.fn(:from_maybe) do
#     on("d", "Nothing") { |d| d }
#     on("_", "Just x")  { |x| x }
#   end
#
#   from_maybe.(0, Just.new(5))   # => 5
#   from_maybe.(0, Nothing)       # => 0
module HaskellMatch
end

require_relative "haskell_match/version"
require_relative "haskell_match/errors"
require_relative "haskell_match/native_loader"

HaskellMatch::NativeLoader.load!

require_relative "haskell_match/inspect"
require_relative "haskell_match/data"
require_relative "haskell_match/lazy_list"
require_relative "haskell_match/pattern_ast"
require_relative "haskell_match/binding_plan"
require_relative "haskell_match/clauses"
require_relative "haskell_match/deep_call"
require_relative "haskell_match/function"
require_relative "haskell_match/case_of"
require_relative "haskell_match/pattern"
require_relative "haskell_match/dsl"
