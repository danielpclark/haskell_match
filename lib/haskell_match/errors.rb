# frozen_string_literal: true

module HaskellMatch
  # Base class for every error raised by haskell_match.
  class Error < StandardError; end

  # Raised while compiling patterns (definition time).
  class CompileError < Error; end

  # Malformed pattern text.
  class PatternSyntaxError < CompileError
    # Reformat the native "msg (column N in \"src\")" into a message that
    # shows the pattern with a caret under the column.
    def self.with_caret(message)
      m = message.match(/\A(.*) \(column (\d+) in (".*")\)\z/m)
      return message unless m

      src = begin
        eval(m[3]) # rubocop:disable Security/Eval -- a Rust-escaped string literal
      rescue SyntaxError, StandardError
        return message
      end
      col = m[2].to_i
      "#{m[1]}\n    #{src}\n    #{' ' * [col - 1, 0].max}^"
    end
  end

  # A constructor name that no registered data type declares.
  class UnknownConstructorError < CompileError; end

  # A constructor applied to the wrong number of arguments.
  class ArityError < CompileError; end

  # Patterns of different types in the same position.
  class PatternTypeError < CompileError; end

  # The same variable bound twice in one clause.
  class DuplicateVariableError < CompileError; end

  # Record pattern problems (unknown field, positional constructor, ...).
  class FieldError < CompileError; end

  # Malformed `data` declaration.
  class DataDeclarationError < CompileError; end

  # Clauses of one function with different numbers of arguments.
  class ClauseArityError < CompileError; end

  # The clauses do not cover every possible value (GHC: -Wincomplete-patterns).
  class NonExhaustiveError < CompileError
    attr_reader :missing

    def initialize(message, missing = [])
      super(message)
      @missing = missing
    end
  end

  # A clause can never be selected (GHC: -Woverlapping-patterns).
  class RedundantClauseError < CompileError
    attr_reader :clauses

    def initialize(message, clauses = [])
      super(message)
      @clauses = clauses
    end
  end

  # Problems with the Ruby side of a definition (bad block parameters, ...).
  class DefinitionError < CompileError; end

  # Raised at match time.
  class MatchError < Error; end

  # A value is not of the type the patterns expect (what Haskell's type
  # checker would have rejected statically).
  class TypeMismatchError < MatchError; end

  # An irrefutable (`~`) pattern failed to destructure after its clause was
  # selected.
  class IrrefutablePatternError < MatchError; end

  # Recursion through compiled functions exceeded `HaskellMatch.max_depth`.
  class StackOverflowError < Error; end
end
