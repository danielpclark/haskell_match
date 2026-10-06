# frozen_string_literal: true

require "monitor"

module HaskellMatch
  # `case ... of` expressions.
  #
  #   HaskellMatch.case_of(value) do
  #     on("Just x") { |x| x }
  #     on("Nothing") { 0 }
  #   end
  #
  # The decision tree for a given set of patterns is compiled once and cached,
  # so repeated evaluation only pays for collecting the clause blocks.
  module CaseOf
    CACHE = {}
    LOCK = Monitor.new
    MAX_CACHE = 4096

    module_function

    def evaluate(values, exhaustive:, overlapping:, &definition)
      raise ArgumentError, "case_of needs a block with on(...) clauses" unless definition
      raise ArgumentError, "case_of needs at least one value" if values.empty?

      clauses = ClauseBuilder.collect(definition)
      key = cache_key(clauses, exhaustive, overlapping)
      entry = LOCK.synchronize { CACHE[key] }
      unless entry
        name = "case expression at #{definition.source_location&.join(':')}"
        matcher, _bodies, _guards = Compiler.compile(name, clauses, exhaustive: exhaustive,
                                                                     overlapping: overlapping, kind: "case expression")
        entry = [matcher, matcher.names]
        LOCK.synchronize do
          CACHE.clear if CACHE.size >= MAX_CACHE
          CACHE[key] = entry
        end
      end
      matcher, names = entry
      bodies = clauses.each_with_index.map { |c, i| BindingPlan.adapt(c.body, names[i], i, "body", "case") }
      guards = clauses.each_with_index.map do |c, i|
        next nil if c.guard.nil? || c.guard.equal?(ClauseBuilder::OTHERWISE)

        BindingPlan.adapt(c.guard, names[i], i, "guard", "case")
      end
      guards = nil if guards.all?(&:nil?)
      matcher.run(values, bodies, guards)
    end

    def cache_key(clauses, exhaustive, overlapping)
      [
        clauses.map { |c| [c.patterns, !c.guard.nil? && !c.guard.equal?(ClauseBuilder::OTHERWISE)] },
        exhaustive, overlapping
      ]
    end

    def clear_cache
      LOCK.synchronize { CACHE.clear }
    end
  end

  class << self
    # Evaluate a `case ... of` expression over one or more values.
    def case_of(*values, exhaustive: self.exhaustive, overlapping: self.overlapping, &definition)
      CaseOf.evaluate(values, exhaustive: exhaustive, overlapping: overlapping, &definition)
    end
  end
end
