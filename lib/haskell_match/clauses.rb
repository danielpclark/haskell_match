# frozen_string_literal: true

module HaskellMatch
  # One `on(...)` clause: argument patterns, optional guard, body.
  Clause = Struct.new(:patterns, :guard, :body, :location) do
    def to_s
      patterns.join(" ")
    end
  end

  # Collects clauses from a definition block.
  #
  #   on("[]")       { 0 }
  #   on("(_:xs)")   { |xs| 1 + length.(xs) }
  #   on("n", guard: ->(n) { n > 0 }) { :positive }
  #
  # The definition block runs with `self` set to the builder; method calls the
  # builder does not understand are forwarded to the object that owns the
  # block, so helper methods of the surrounding class or test remain
  # callable.  Instance variables of that object are not visible; take the
  # builder as a block parameter (`fn { |m| m.on(...) { @x } }`) when needed.
  class ClauseBuilder
    attr_reader :clauses

    def initialize(owner = nil)
      @clauses = []
      @owner = owner
    end

    def method_missing(name, *args, **kwargs, &blk)
      if @owner && @owner.respond_to?(name, true)
        @owner.__send__(name, *args, **kwargs, &blk)
      else
        super
      end
    end

    def respond_to_missing?(name, include_private = false)
      (@owner && @owner.respond_to?(name, include_private)) || super
    end

    # Run a definition block: instance_exec'd on the builder when it takes no
    # parameters, otherwise called with the builder as its argument.
    #
    # In `ractor` mode the builder has no owner to forward to, and is frozen
    # and emptied once the clauses are collected, so the clause procs (whose
    # `self` it is) can be passed to `Ractor.make_shareable`.
    def self.collect(definition, ractor: false)
      builder = new(ractor ? nil : definition.binding.receiver)
      if definition.arity.zero?
        builder.instance_exec(&definition)
      else
        definition.call(builder)
      end
      clauses = builder.clauses
      builder.detach! if ractor
      clauses
    end

    def detach!
      @clauses = nil
      @owner = nil
      freeze
    end

    # Add a clause.  Patterns are Haskell pattern strings, one per argument.
    def on(*patterns, guard: nil, where: nil, &body)
      guard ||= where
      raise DefinitionError, "a clause needs at least one pattern" if patterns.empty?
      raise DefinitionError, "a clause needs a body block" unless body
      if guard && !guard.respond_to?(:call)
        raise DefinitionError, "guard must be callable (a Proc or lambda)"
      end

      patterns.each do |p|
        raise DefinitionError, "patterns must be Strings, got #{p.inspect}" unless p.is_a?(String)
      end
      @clauses << Clause.new(patterns.map(&:dup).each(&:freeze).freeze, guard, body, body.source_location)
      self
    end
    alias clause on
    alias _ on

    # Haskell's `otherwise`: a guard that always passes.
    def otherwise
      OTHERWISE
    end

    OTHERWISE = ->(*) { true }.freeze
  end

  # Compiles clauses into a native matcher and enforces exhaustiveness and
  # redundancy policy.  Shared by {Function}, {CaseOf} and {Pattern}.
  module Compiler
    WITNESS_LIMIT = 20

    POLICIES = %i[error warn ignore].freeze

    module_function

    def policy(value, option)
      case value
      when true then :error
      when false, nil then :ignore
      when :error, :warn, :ignore then value
      else
        raise ArgumentError, "#{option}: must be true, false, :error, :warn or :ignore (got #{value.inspect})"
      end
    end

    # Returns [matcher, bodies, guards] where bodies/guards are arrays of
    # callables taking the bound values positionally.
    def compile(name, clauses, exhaustive:, overlapping:, kind: "equation", klass: Native::Matcher)
      raise DefinitionError, "'#{name}' has no clauses" if clauses.empty?

      exhaustive = policy(exhaustive, :exhaustive)
      overlapping = policy(overlapping, :overlapping)
      matcher = klass.new(
        name.to_s,
        clauses.map(&:patterns),
        clauses.map { |c| !c.guard.nil? && !c.guard.equal?(ClauseBuilder::OTHERWISE) },
        WITNESS_LIMIT
      )
      report(name, kind, clauses, matcher, exhaustive, overlapping)

      names = matcher.names
      bodies = clauses.each_with_index.map do |c, i|
        BindingPlan.adapt(c.body, names[i], i, "body", name)
      end
      guards = clauses.each_with_index.map do |c, i|
        next nil if c.guard.nil? || c.guard.equal?(ClauseBuilder::OTHERWISE)

        BindingPlan.adapt(c.guard, names[i], i, "guard", name)
      end
      guards = nil if guards.all?(&:nil?)
      [matcher, bodies, guards]
    end

    def report(name, kind, clauses, matcher, exhaustive, overlapping)
      redundant = matcher.instance_variable_get(:@redundant)
      if !redundant.empty? && overlapping != :ignore
        lines = redundant.map do |i|
          c = clauses[i]
          loc = c.location ? " (#{c.location.join(':')})" : ""
          "    #{name} #{c} = ...#{loc}"
        end
        message = "Pattern match#{redundant.size > 1 ? 'es are' : ' is'} redundant\n" \
                  "In #{article(kind)} #{kind} for '#{name}':\n#{lines.join("\n")}"
        if overlapping == :error
          raise RedundantClauseError.new(message, redundant)
        else
          warn "haskell_match: #{message}"
        end
      end

      missing = matcher.instance_variable_get(:@missing)
      if !missing.empty? && exhaustive != :ignore
        shown = missing.map { |m| "    #{m.empty? ? '(no clause can match)' : m}" }
        shown << "    ..." if matcher.instance_variable_get(:@missing_truncated)
        message = "Pattern match(es) are non-exhaustive\n" \
                  "In #{article(kind)} #{kind} for '#{name}':\n" \
                  "    Patterns not matched:\n#{shown.map { |s| '    ' + s }.join("\n")}"
        if exhaustive == :error
          raise NonExhaustiveError.new(message, missing)
        else
          warn "haskell_match: #{message}"
        end
      end
    end

    def article(word)
      word.match?(/\A[aeiou]/i) ? "an" : "a"
    end
  end
end
