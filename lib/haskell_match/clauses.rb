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
    include PatternAST::BuilderMethods

    attr_reader :clauses

    def initialize(owner = nil, options = {})
      @clauses = []
      @owner = owner
      @options = options
      @helpers = {}
    end

    # A local helper function, like a Haskell `where` binding: defined with
    # its own clauses and reachable by name from every clause body of the
    # enclosing function (and from the other helpers).  It is a full
    # {Function}: checked for exhaustiveness, with `tail` and `defer`, and
    # compiled with the enclosing function's options unless overridden.
    #
    #   sum_to = HaskellMatch.fn(:sum_to) do
    #     on(n) { |n| go.(n, 0) }
    #     where :go do
    #       on(0, acc) { |acc| acc }
    #       on(k, acc) { |k, acc| go.tail(k - 1, acc + k) }
    #     end
    #   end
    def where(name, **options, &definition)
      raise DefinitionError, "where needs a block of on(...) clauses" unless definition

      name = name.to_s
      raise DefinitionError, "'#{name}' is not a valid helper name" unless name.match?(NAME_PATTERN)
      raise DefinitionError, "helper '#{name}' is defined twice" if @helpers.key?(name)

      helper = HaskellMatch.fn(name, **@options.merge(options), &definition)
      @helpers[name] = helper
      define_singleton_method(name) { helper }
      helper
    end

    # Helper functions defined with {#where}, by name.
    def helpers
      @helpers.dup
    end

    # Resolve a constant the way code at the definition block would see it:
    # through the receiver (a module, or an object's class) and so through
    # its ancestors and `Object`.
    def constant_resolver
      owner = @owner
      lambda do |name|
        home = owner.is_a?(Module) ? owner : owner.class
        home.const_defined?(name) ? home.const_get(name) : nil
      rescue NameError
        nil
      end
    end

    # Add a clause: one pattern per argument, each either a Haskell pattern
    # string or a pattern written in place (see {PatternAST}).
    def on(*patterns, guard: nil, where: nil, location: nil, &body)
      guard ||= where
      raise DefinitionError, "a clause needs at least one pattern" if patterns.empty?
      raise DefinitionError, "a clause needs a body block" unless body
      if guard && !guard.respond_to?(:call)
        raise DefinitionError, "guard must be callable (a Proc or lambda)"
      end

      texts = patterns.map { |p| p.is_a?(String) ? p.dup.freeze : PatternAST.render(p).freeze }
      @clauses << Clause.new(texts.freeze, guard, body, location || body.source_location)
      self
    end
    alias clause on

    # Haskell's `otherwise`: a guard that always passes.
    def otherwise
      OTHERWISE
    end

    OTHERWISE = ->(*) { true }.freeze

    # The function being defined, for recursion from inside its own clause
    # bodies (also reachable under the function's own name).
    def recur
      @function or raise DefinitionError, "the function is not defined yet"
    end
    alias this recur

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

    def pattern_owner_responds?(name)
      !@owner.nil? && @owner.respond_to?(name, true)
    end

    # Run a definition block: instance_exec'd on the builder when it takes no
    # parameters, otherwise called with the builder as its argument.  Returns
    # the builder; call {#define_function} once the function exists so clause
    # bodies can recurse through `recur` or the function's name.
    def self.collect(definition, ractor: false, options: {})
      builder = ractor ? RactorHost.new_builder(options) : new(definition.binding.receiver, options)
      if definition.arity.zero?
        builder.instance_exec(&definition)
      else
        definition.call(builder)
      end
      builder
    end

    NAME_PATTERN = /\A[a-z_][A-Za-z0-9_]*[?!]?\z/

    # Make the finished function reachable from its clause bodies as `recur`
    # and, when the name is a valid method name, as `name`.
    def define_function(function, name)
      @function = function
      return unless name.is_a?(String) && name.match?(NAME_PATTERN) && !real_method?(name)

      define_singleton_method(name) { function }
    end

    # A method actually defined on the builder (not one `method_missing`
    # would synthesise as a pattern variable).
    def real_method?(name)
      singleton_class.method_defined?(name) || singleton_class.private_method_defined?(name)
    end

    # Clause-builder behaviour for Ractor-shareable functions.  A Module is
    # used as the builder (and so as `self` of the clause bodies) because
    # modules are always shareable while remaining open for a constant that
    # points back at the finished function.
    module RactorHost
      include PatternAST::BuilderMethods

      def self.new_builder(options = {})
        host = Module.new
        host.extend(RactorHost)
        host.instance_variable_set(:@clauses, [])
        host.instance_variable_set(:@options, options)
        host.instance_variable_set(:@helpers, {})
        host
      end

      def clauses
        @clauses
      end

      # A `where` helper (see {ClauseBuilder#where}); in Ractor mode helpers
      # are themselves shareable functions and cannot call back into the
      # enclosing function by name.
      def where(name, **options, &definition)
        raise DefinitionError, "where needs a block of on(...) clauses" unless definition

        name = name.to_s
        raise DefinitionError, "'#{name}' is not a valid helper name" unless name.match?(NAME_PATTERN)
        raise DefinitionError, "helper '#{name}' is defined twice" if @helpers.key?(name)

        helper = HaskellMatch.fn(name, **@options.merge(options), ractor: true, &definition)
        @helpers[name] = helper
        define_singleton_method(name, &Ractor.make_shareable(-> { helper }))
        helper
      end

      def helpers
        @helpers.dup
      end

      def on(*patterns, guard: nil, where: nil, location: nil, &body)
        guard ||= where
        raise DefinitionError, "a clause needs at least one pattern" if patterns.empty?
        raise DefinitionError, "a clause needs a body block" unless body
        if guard && !guard.respond_to?(:call)
          raise DefinitionError, "guard must be callable (a Proc or lambda)"
        end

        texts = patterns.map { |p| p.is_a?(String) ? p.dup.freeze : PatternAST.render(p).freeze }
        @clauses << Clause.new(texts.freeze, guard, body, location || body.source_location)
        self
      end
      alias clause on

      def otherwise
        OTHERWISE
      end

      # Constant lookup is the Ractor-safe way back to the function.
      def recur
        const_get(:FUNCTION)
      end
      alias this recur

      def define_function(function, name)
        const_set(:FUNCTION, function)
        return if !name.is_a?(String) || !name.match?(NAME_PATTERN)
        return if singleton_class.method_defined?(name) || singleton_class.private_method_defined?(name)

        host = self
        define_singleton_method(name, &Ractor.make_shareable(-> { host.const_get(:FUNCTION) }))
      end
    end
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

    # Construct a native matcher, registering top-level Ruby `Data`/`Struct`
    # classes named by unknown constructors on the way and adding a caret to
    # syntax errors.
    def build_matcher(klass, name, patterns, guard_flags, limit, scope, resolver: nil)
      tried = []
      begin
        klass.new(name, patterns, guard_flags, limit, scope)
      rescue UnknownConstructorError => e
        con = e.message[/data constructor '([^']+)'/, 1]
        raise if con.nil? || tried.include?(con) || !autoregister(con, scope, resolver)

        tried << con
        retry
      rescue PatternSyntaxError => e
        raise e.exception(PatternSyntaxError.with_caret(e.message)), cause: nil
      end
    end

    # A pattern named a constructor nobody declared: if a Ruby `Data` or
    # `Struct` class of that name is visible (through `resolver`, which sees
    # what the definition block sees, or at top level), it becomes a type of
    # its own (`HaskellMatch.sealed`) so plain Ruby value classes match
    # without a declaration.  Returns whether a type was registered.
    def autoregister(name, scope, resolver = nil)
      return false if HaskellMatch.constructor(name, scope)

      klass = resolver&.call(name)
      klass = Object.const_get(name) if klass.nil? && Object.const_defined?(name)
      return false unless klass.is_a?(Class) && (klass < Data || klass < Struct)
      return false if klass.singleton_class.include?(Constructor::SealedClassMethods) || klass.include?(Constructor)

      HaskellMatch.sealed(name, klass, under: nil, scope: scope)
      true
    rescue NameError, CompileError
      false
    end

    # Returns [matcher, bodies, guards] where bodies/guards are arrays of
    # callables taking the bound values positionally.
    def compile(name, clauses, exhaustive:, overlapping:, kind: "equation", klass: Native::Matcher,
                scope: Native::GLOBAL_SCOPE, resolver: nil)
      raise DefinitionError, "'#{name}' has no clauses" if clauses.empty?

      exhaustive = policy(exhaustive, :exhaustive)
      overlapping = policy(overlapping, :overlapping)
      matcher = build_matcher(
        klass,
        name.to_s,
        clauses.map(&:patterns),
        clauses.map { |c| !c.guard.nil? && !c.guard.equal?(ClauseBuilder::OTHERWISE) },
        WITNESS_LIMIT,
        scope,
        resolver: resolver
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
