# frozen_string_literal: true

module HaskellMatch
  # A function defined by pattern-matching clauses.
  #
  #   length = HaskellMatch.fn(:length) do
  #     on("[]")     { 0 }
  #     on("(_:xs)") { |xs| 1 + length.(xs) }
  #   end
  #   length.([1, 2, 3])  # => 3
  # Returned by {Function#tail} and {Function#defer}: tells the native `call`
  # to continue with `function` applied to `args`, and (for `defer`) to pass
  # the eventual result to `continuation`.  The native call keeps pending
  # continuations on its own stack, so recursion written this way is bounded
  # by memory rather than by Ruby's VM stack.
  TailCall = Data.define(:function, :args, :continuation)

  class Function < Native::Matcher
    attr_reader :clauses

    # Request a tail call: `recur.tail(n - 1, acc * n)` as the last
    # expression of a clause body re-enters the function (or any other
    # function) in constant stack space, like a tail call in Haskell.
    def tail(*args)
      TailCall.new(self, args, nil)
    end

    # Request a non-tail call whose result the block will receive:
    # `length.defer(xs) { |n| 1 + n }` stands for `1 + length xs` and runs
    # with the pending work kept off Ruby's stack, however deep it goes.
    def defer(*args, &continuation)
      raise ArgumentError, "defer needs a block to receive the result" unless continuation

      TailCall.new(self, args, continuation)
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
    #
    # With `deep: true` the function's `call` is implemented in Ruby (see
    # {DeepCall}): about 100 ns slower per call, but deep recursion costs a
    # tenth of the memory and the GC scans a fifth as much.  The default is
    # {HaskellMatch.deep_by_default}.
    def self.define(name, builder, exhaustive:, overlapping:, ractor: false, deep: HaskellMatch.deep_by_default,
                    scope: Native::GLOBAL_SCOPE)
      clauses = builder.clauses
      clauses = make_shareable(clauses) if ractor
      f, bodies, guards = Compiler.compile(name.to_s, clauses, exhaustive: exhaustive,
                                                             overlapping: overlapping, klass: self, scope: scope)
      f.instance_variable_set(:@scope, scope)
      f.extend(DeepCall.module_for(f.arity)) if deep
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

    # call(*args) is native (or Ruby, in deep mode); these make a Function
    # behave like a Proc.
    def [](*args)
      call(*args)
    end

    def ===(*args)
      call(*args)
    end

    # Whether this function runs in deep mode (see {DeepCall}).
    def deep?
      false
    end

    # A lambda with the function's exact arity.  Built on first use (a
    # shareable function cannot hold a reference back to itself).
    def to_proc
      return ProcBuilder.build(self, arity) if frozen?

      @proc ||= ProcBuilder.build(self, arity)
    end

    def curry
      to_proc.curry(arity)
    end

    # The function as a curried Proc (memoised); what Haskell code receives
    # when it passes a function as a value.
    def curried
      return to_proc if arity == 1
      return @curried if defined?(@curried) && @curried

      c = to_proc.curry(arity)
      @curried = c unless frozen?
      c
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

    # Every this many nested calls the next clause body runs in a fresh Fiber
    # (with its own VM and machine stacks), so plain recursion is bounded by
    # memory rather than by Ruby's stack size.  0 disables this.
    def stack_segment
      Native.stack_segment
    end

    def stack_segment=(levels)
      Native.stack_segment = levels
      DeepCall.segment = Native.stack_segment
    end

    # Whether functions use deep mode unless told otherwise (default false).
    attr_writer :deep_by_default

    def deep_by_default
      @deep_by_default ? true : false
    end

    # Deepest allowed nesting of calls (0 = unlimited); beyond it
    # {StackOverflowError} is raised rather than letting a runaway recursion
    # take all memory.
    def max_depth
      Native.max_depth
    end

    def max_depth=(levels)
      Native.max_depth = levels
    end

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
    # * `deep:`        true for Ruby-level body invocation (cheap deep recursion)
    # * `scope:`       the type scope the patterns resolve constructors in (see {HaskellMatch.new_scope})
    def fn(name = nil, exhaustive: self.exhaustive, overlapping: self.overlapping, ractor: false,
           deep: deep_by_default, scope: Native::GLOBAL_SCOPE, &definition)
      raise ArgumentError, "HaskellMatch.fn needs a block with on(...) clauses" unless definition

      builder = ClauseBuilder.collect(definition, ractor: ractor)
      name ||= "anonymous function at #{definition.source_location&.join(':')}"
      Function.define(name, builder, exhaustive: exhaustive, overlapping: overlapping, ractor: ractor, deep: deep,
                                     scope: scope)
    end
  end
end
