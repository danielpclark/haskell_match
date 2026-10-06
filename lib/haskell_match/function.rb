# frozen_string_literal: true

module HaskellMatch
  # A function defined by pattern-matching clauses.
  #
  #   length = HaskellMatch.fn(:length) do
  #     on("[]")     { 0 }
  #     on("(_:xs)") { |xs| 1 + length.(xs) }
  #   end
  #   length.([1, 2, 3])  # => 3
  # Returned by {Function#tail}: tells the native `call` to continue with
  # `function` applied to `args` instead of growing the stack.
  TailCall = Data.define(:function, :args)

  class Function < Native::Matcher
    attr_reader :clauses

    # Request a tail call: `recur.tail(n - 1, acc * n)` as the last
    # expression of a clause body re-enters the function (or any other
    # function) in constant stack space, like a tail call in Haskell.
    def tail(*args)
      TailCall.new(self, args)
    end

    # Build a function from clauses (use {HaskellMatch.fn}).  The native
    # `new` compiles the patterns; the clause bodies and guards are then
    # attached as instance variables the native `call` reads on every call.
    #
    # With `ractor: true` the clause bodies and guards are made shareable
    # (`Ractor.make_shareable`), as is the function, so it can be sent to and
    # called from other Ractors.  In that mode the definition block cannot
    # call methods of the surrounding object, and local variables captured
    # by the bodies must already be shareable; recursion goes through the
    # function's name or `recur`, which work in both modes.
    def self.define(name, builder, exhaustive:, overlapping:, ractor: false)
      clauses = builder.clauses
      clauses = make_shareable(clauses) if ractor
      f, bodies, guards = Compiler.compile(name.to_s, clauses, exhaustive: exhaustive,
                                                             overlapping: overlapping, klass: self)
      f.send(:attach, clauses, bodies, guards)
      builder.define_function(f, name.to_s)
      Ractor.make_shareable(f) if ractor
      f
    end

    def self.make_shareable(clauses)
      raise DefinitionError, "ractor: true requires Ractor support in this Ruby" unless defined?(Ractor)

      clauses.map do |c|
        body = Ractor.make_shareable(c.body)
        guard = if c.guard.nil? || c.guard.equal?(ClauseBuilder::OTHERWISE)
                  c.guard
                else
                  Ractor.make_shareable(c.guard)
                end
        Clause.new(c.patterns, guard, body, c.location)
      end
    end
    private_class_method :make_shareable

    # call(*args) is native; these aliases make a Function behave like a Proc.
    alias [] call
    alias === call

    # A lambda with the function's exact arity.  Built on first use (a
    # shareable function cannot hold a reference back to itself).
    def to_proc
      return ProcBuilder.build(self, arity) if frozen?

      @proc ||= ProcBuilder.build(self, arity)
    end

    def curry
      to_proc.curry(arity)
    end

    # Variables bound by each clause, in order.
    def bindings
      names
    end

    # Human-readable dump of the compiled decision tree.
    def decision_tree
      tree
    end

    def inspect
      "#<HaskellMatch::Function #{name}/#{arity}>"
    end
    alias to_s inspect

    private

    def attach(clauses, bodies, guards)
      @clauses = clauses.freeze
      @bodies = bodies.freeze
      @guards = guards&.freeze
      @proc = nil
    end

    # Builds the lambda returned by {Function#to_proc}.  It is created here,
    # rather than inside the function, so its `self` is this (shareable)
    # module and `Ractor.make_shareable` accepts it.
    module ProcBuilder
      def self.build(function, arity)
        params = (1..arity).map { |i| "a#{i}" }.join(", ")
        # a lambda with the function's exact arity
        eval("->(#{params}) { function.call(#{params}) }", binding, __FILE__, __LINE__) # rubocop:disable Style/EvalWithLocation
      end
    end
  end

  class << self
    attr_writer :exhaustive, :overlapping

    # Default policy for non-exhaustive clause sets: :error (Haskell's
    # behaviour, the default), :warn or :ignore.
    def exhaustive
      @exhaustive.nil? ? :error : @exhaustive
    end

    # Default policy for redundant clauses: :error (default), :warn or :ignore.
    def overlapping
      @overlapping.nil? ? :error : @overlapping
    end

    # Define a function by clauses.  See {Function}.
    #
    # Options:
    # * `exhaustive:`  true/:error (default), :warn, or false/:ignore
    # * `overlapping:` true/:error (default), :warn, or false/:ignore
    # * `ractor:`      true to make the function Ractor-shareable (see {Function.define})
    def fn(name = nil, exhaustive: self.exhaustive, overlapping: self.overlapping, ractor: false, &definition)
      raise ArgumentError, "HaskellMatch.fn needs a block with on(...) clauses" unless definition

      builder = ClauseBuilder.collect(definition, ractor: ractor)
      name ||= "anonymous function at #{definition.source_location&.join(':')}"
      Function.define(name, builder, exhaustive: exhaustive, overlapping: overlapping, ractor: ractor)
    end
  end
end
