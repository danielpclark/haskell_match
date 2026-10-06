# frozen_string_literal: true

module HaskellMatch
  # A single compiled pattern, usable on its own:
  #
  #   p = HaskellMatch.pattern("Just (x:_)")
  #   p.match(Just.new([1, 2]))   # => { x: 1 }
  #   p.match(Nothing)            # => nil
  #   p === Just.new([1])         # => true   (so it works in case/when)
  class Pattern
    attr_reader :source, :names

    def initialize(source = nil, scope: Native::GLOBAL_SCOPE, &block)
      if block
        raise ArgumentError, "pass either a pattern string or a block, not both" if source

        source = PatternAST.render(ClauseBuilder.new(block.binding.receiver).instance_exec(&block))
      end
      raise ArgumentError, "pattern must be a String" unless source.is_a?(String)

      @source = source.dup.freeze
      @scope = scope
      @matcher = Native::Matcher.new("pattern #{source.inspect}", [[source]], [false], 1, scope)
      @names = @matcher.names.first.map(&:to_sym).freeze
      @exhaustive = @matcher.instance_variable_get(:@missing).empty?
      freeze
    end

    # Bindings as a Hash, or nil when the value does not match.  Raises
    # {TypeMismatchError} when the value is of the wrong type altogether.
    def match(value)
      result = @matcher.select(value)
      return nil unless result

      @names.zip(result[1]).to_h
    end

    # Like {#match} but raises {MatchError} on failure.
    def match!(value)
      match(value) or raise MatchError, "#{value.inspect} does not match pattern #{@source}"
    end

    # Does the value match?  Unlike {#match}, a value of the wrong type is
    # simply a non-match, so the pattern can be used in `case`/`when`.
    def ===(value)
      !@matcher.select(value).nil?
    rescue TypeMismatchError
      false
    end

    # Whether the pattern matches every value of its type (irrefutable).
    def irrefutable?
      @exhaustive
    end

    def to_proc
      method(:match).to_proc
    end

    def inspect
      "#<HaskellMatch::Pattern #{@source}>"
    end
    alias to_s inspect
  end

  class << self
    # `HaskellMatch.pattern("Just (x:_)")` or `HaskellMatch.pattern { Just([x, *_]) }`
    def pattern(source = nil, scope: Native::GLOBAL_SCOPE, &block)
      Pattern.new(source, scope: scope, &block)
    end
    alias [] pattern
  end
end
