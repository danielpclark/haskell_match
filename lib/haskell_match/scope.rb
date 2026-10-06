# frozen_string_literal: true

module HaskellMatch
  # Type scopes.  Scope 0 is the global registry that `HaskellMatch.data`
  # fills by default and every pattern compiles against unless told
  # otherwise.  A compiled Haskell module gets a scope of its own
  # ({Haskell#haskell_scope}): a snapshot of the global registry at the time
  # it is created, plus the module's own `data` declarations, so two modules
  # can each declare a `Shape` without interfering.
  module Native
    GLOBAL_SCOPE = 0

    class << self
      def register_type(name, specs, scope = GLOBAL_SCOPE)
        register_type_in(name, specs, scope)
      end

      def render_pattern(source, scope = GLOBAL_SCOPE)
        render_pattern_in(source, scope)
      end

      def constructors(type_name, scope = GLOBAL_SCOPE)
        constructors_in(type_name, scope)
      end

      def constructor_info(name, scope = GLOBAL_SCOPE)
        constructor_info_in(name, scope)
      end
    end
  end

  class << self
    # A fresh type scope seeded from the global registry.
    def new_scope
      Native.new_scope
    end
  end
end
