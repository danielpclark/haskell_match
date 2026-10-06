# frozen_string_literal: true

module HaskellMatch
  # Works out how a clause body (or guard) wants its bound variables and
  # produces a callable taking them positionally, in pattern order, as the
  # native matcher delivers them.
  #
  # * No parameters, or a splat: the values are passed positionally.
  # * Positional parameters: each must name a bound variable; values are
  #   passed by name, so `on("(x:xs)") { |xs| ... }` receives `xs`.
  # * Keyword parameters: passed by name as keywords.
  module BindingPlan
    module_function

    # Returns a callable that accepts the bound values positionally.
    def adapt(callable, names, clause_index, role, function_name)
      params = callable.parameters
      if params.empty?
        # a lambda taking nothing must not be handed the bound values
        return callable unless callable.lambda? && !names.empty?

        return ->(*) { callable.call }
      end

      kinds = params.map(&:first)
      if kinds.include?(:keyreq) || kinds.include?(:key) || kinds.include?(:keyrest)
        return adapt_keywords(callable, params, names, clause_index, role, function_name)
      end
      return callable if kinds.include?(:rest)
      if kinds.include?(:block)
        params = params.reject { |k, _| k == :block }
        return callable if params.empty?
      end

      wanted = params.map { |_, n| n }
      if wanted.any?(&:nil?)
        # e.g. `|(a, b)|` destructuring parameters: pass positionally
        return callable
      end
      unknown = wanted.reject { |n| names.include?(n.to_s) }
      unless unknown.empty?
        raise DefinitionError,
              "#{role} of clause #{clause_index + 1} of '#{function_name}' names " \
              "#{unknown.map(&:inspect).join(', ')} but the pattern binds " \
              "#{names.empty? ? 'no variables' : names.join(', ')}"
      end
      indexes = wanted.map { |n| names.index(n.to_s) }
      return callable if indexes == (0...indexes.size).to_a && indexes.size == names.size
      return callable if indexes == (0...indexes.size).to_a && !callable.lambda?

      lambda do |*values|
        callable.call(*indexes.map { |i| values[i] })
      end
    end

    def adapt_keywords(callable, params, names, clause_index, role, function_name)
      positional = params.select { |k, _| %i[req opt].include?(k) }
      unless positional.empty?
        raise DefinitionError,
              "#{role} of clause #{clause_index + 1} of '#{function_name}' mixes positional and " \
              "keyword parameters; use one style"
      end
      keys = params.select { |k, _| %i[keyreq key].include?(k) }.map { |_, n| n }
      unknown = keys.reject { |n| names.include?(n.to_s) }
      unless unknown.empty?
        raise DefinitionError,
              "#{role} of clause #{clause_index + 1} of '#{function_name}' names keyword(s) " \
              "#{unknown.map(&:inspect).join(', ')} but the pattern binds " \
              "#{names.empty? ? 'no variables' : names.join(', ')}"
      end
      symbols = names.map(&:to_sym)
      if params.any? { |k, _| k == :keyrest }
        lambda do |*values|
          callable.call(**symbols.zip(values).to_h)
        end
      else
        picks = keys.map { |k| [k, names.index(k.to_s)] }
        lambda do |*values|
          callable.call(**picks.to_h { |k, i| [k, values[i]] })
        end
      end
    end
  end
end
