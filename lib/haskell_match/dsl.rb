# frozen_string_literal: true

module HaskellMatch
  # Mixin providing `fn`, `case_of`, `pattern`, `data` and `hdef`.
  #
  #   class Calculator
  #     extend HaskellMatch::DSL
  #     include Maybe
  #
  #     hdef :or_zero do
  #       on("Just x")  { |x| x }
  #       on("Nothing") { 0 }
  #     end
  #   end
  module DSL
    def fn(name = nil, **options, &definition)
      HaskellMatch.fn(name, **options, &definition)
    end

    def case_of(*values, **options, &definition)
      HaskellMatch.case_of(*values, **options, &definition)
    end

    def pattern(source)
      HaskellMatch.pattern(source)
    end

    def data(decl, **options)
      HaskellMatch.data(decl, **options)
    end

    # Define an instance method (when extended onto a class or module) or a
    # singleton method (when `self` is any other object) by clauses.  Clause
    # bodies and guards run with `self` set to the receiver.
    def hdef(name, exhaustive: HaskellMatch.exhaustive, overlapping: HaskellMatch.overlapping, &definition)
      raise ArgumentError, "hdef needs a block with on(...) clauses" unless definition

      clauses = ClauseBuilder.collect(definition).clauses
      matcher, _bodies, _guards = Compiler.compile(name, clauses, exhaustive: exhaustive, overlapping: overlapping)
      names = matcher.names
      guards = clauses.map { |c| c.guard.equal?(ClauseBuilder::OTHERWISE) ? nil : c.guard }
      plans = clauses.each_with_index.map do |c, i|
        [BindingPlan.adapt(c.body, names[i], i, "body", name),
         (guards[i] && BindingPlan.adapt(guards[i], names[i], i, "guard", name))]
      end
      has_guards = guards.any?

      impl = lambda do |receiver, args|
        guard_procs = nil
        if has_guards
          guard_procs = plans.map do |_, g|
            next nil unless g

            ->(*vals) { receiver.instance_exec(*vals, &g) }
          end
        end
        index, values = matcher.select_with(args, guard_procs)
        raise MatchError, "Non-exhaustive patterns in #{name}" if index.nil?

        receiver.instance_exec(*values, &plans[index][0])
      end

      target = is_a?(Module) ? self : singleton_class
      target.define_method(name) { |*args| impl.call(self, args) }
      name
    end
  end
end
