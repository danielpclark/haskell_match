# frozen_string_literal: true

require "json"

module HaskellMatch
  module Haskell
    # Compiles the JSON AST produced by the native parser into Ruby source
    # that is evaluated in the host module.
    #
    # * Each top-level function becomes a `HaskellMatch.fn` held in an
    #   instance variable of the module (`@hs_name`), plus a singleton method
    #   (`Mod.name` and its snake_case alias) for callers in Ruby.
    # * Values (`x = expr`) become memoised singleton methods.
    # * `where`/`let` functions, lambdas with refutable patterns and `case`
    #   expressions are lambda-lifted into hidden top-level functions whose
    #   leading parameters are their free variables, so every pattern match
    #   is compiled exactly once.
    # * Saturated calls to known functions are direct; a function used as a
    #   value is a curried Proc.  Calls in tail position compile to `tail`,
    #   so Haskell loops run in constant space.
    class Compiler
      Scope = Struct.new(:vars, :funs, :parent) do
        # vars: haskell name => ruby expression (local or param name)
        # funs: haskell name => { ivar:, arity:, free: [haskell names] }
        def lookup_var(name)
          return vars[name] if vars.key?(name)

          parent&.lookup_var(name)
        end

        def lookup_fun(name)
          return funs[name] if funs.key?(name)

          parent&.lookup_fun(name)
        end

        def shadowed?(name)
          vars.key?(name) || funs.key?(name) || (parent&.shadowed?(name) || false)
        end
      end

      BINOPS = {
        "+" => "+", "-" => "-", "*" => "*", "==" => "==", "/=" => "!=",
        "<" => "<", "<=" => "<=", ">" => ">", ">=" => ">=", "&&" => "&&", "||" => "||", "^" => "**"
      }.freeze
      PRELUDE_BINOPS = {
        "/" => "fdiv", "**" => "powf", "++" => "append", ":" => "cons", "!!" => "index", "." => "compose"
      }.freeze

      attr_reader :source_name, :lines, :lifted_lines

      def initialize(ast, host, source_name: "haskell", exhaustive: HaskellMatch.exhaustive, line_offset: 0,
                     scope: Native::GLOBAL_SCOPE)
        @ast = ast
        @line_offset = line_offset
        @scope = scope
        @host = host
        @source_name = source_name
        @exhaustive = exhaustive
        @lifted = [] # generated definitions (strings), in order
        @counter = 0
        @top = {} # name => { ivar:, arity: } for top-level functions
        @values = {} # name => method name for top-level values
        @con_arity = {} # constructor name => arity
        @local_cons = [] # constructors declared by this module
        @local_types = [] # type names declared by this module
        @lines = {} # function name => source line of its first equation
        @lifted_lines = {} # the same for where/let-bound functions
        # functions and values the host already has (imports, or an earlier
        # `haskell` call on the same module) are callable like local ones
        imports = host.respond_to?(:haskell_imports) ? host.haskell_imports : {}
        if host.respond_to?(:haskell_functions)
          host.haskell_functions.each do |n, f|
            @top[n] = { ivar: "@__haskell_functions__[#{rb_str(n)}]", arity: f.arity }
          end
        end
        imports.each { |n, kind| @values[n] = n if kind == :value && !@top.key?(n) }
      end

      # Generate the Ruby source for the module.
      def generate
        decls = @ast.fetch("decls")
        out = []
        # data declarations first: patterns need the constructors
        decls.each do |d|
          next unless d["kind"] == "data"

          out << data_decl(d)
        end
        decls.each do |d|
          case d["kind"]
          when "fun"
            @top[d["name"]] = { ivar: ivar_for(d["name"]), arity: d["arity"] }
            @lines[d["name"]] ||= ln(d["equations"].first&.fetch("line", nil))
          when "bind"
            pat = d["pat"]
            unless pat["vars"].size == 1 && pat["text"] == mangle(pat["vars"][0])
              raise DefinitionError, "#{@source_name}:#{ln(d['line'])}: only simple names can be bound at top level (got #{pat['text']})"
            end
            @values[pat["vars"][0]] = "hs_value_#{rb_ident(pat['vars'][0])}"
          end
        end
        body = []
        decls.each do |d|
          case d["kind"]
          when "fun" then body << function(d)
          when "bind" then body << value(d)
          end
        end
        table = @top.map { |n, info| "#{rb_str(n)} => #{info[:ivar]}" }.join(", ")
        body << "(@__haskell_functions__ ||= {}).merge!({ #{table} })"
        values = @values.map { |n, m| "#{rb_str(n)} => #{m.to_sym.inspect}" }.join(", ")
        body << "(@__haskell_values__ ||= {}).merge!({ #{values} })"
        body << "(@__haskell_types__ ||= []).concat(#{@local_types.inspect}).uniq!"
        body << "@__haskell_exports__ = #{@ast['exports'].inspect}" if @ast["exports"]
        (out + @lifted + body).join("\n")
      end

      private

      # A Haskell source line as a line of the enclosing file.
      def ln(line)
        line && line + @line_offset
      end

      def mangle(name)
        "hs_" + name.gsub("'", "_q")
      end

      def fresh(prefix)
        @counter += 1
        "#{prefix}_#{@counter}"
      end

      def ivar_for(name)
        "@hs_fn_#{rb_ident(name)}"
      end

      SYMBOL_WORDS = {
        ":" => "colon", "+" => "plus", "-" => "minus", "*" => "star", "/" => "slash", "<" => "lt", ">" => "gt",
        "=" => "eq", "!" => "bang", "@" => "at", "#" => "hash", "$" => "dollar", "%" => "percent", "&" => "amp",
        "^" => "caret", "|" => "bar", "~" => "tilde", "?" => "query", "." => "dot", "\\" => "backslash"
      }.freeze

      # A Haskell function or operator name as a Ruby identifier fragment.
      def rb_ident(name)
        return name.gsub("'", "_q") if name.match?(/\A[A-Za-z_]/)

        "op_" + name.chars.map { |c| SYMBOL_WORDS.fetch(c) { "u#{c.ord}" } }.join("_")
      end

      def symbolic?(name)
        !name.match?(/\A[A-Za-z_]/)
      end

      # A user-defined function or operator visible from `scope`.
      def user_function?(name, scope)
        !scope.lookup_fun(name).nil? || @top.key?(name) || @values.key?(name)
      end

      def snake(name)
        name.gsub(/([a-z\d])([A-Z])/, '\1_\2').downcase.gsub("'", "_prime")
      end

      def rb_str(s)
        s.inspect
      end

      # ------------------------------------------------------------ declarations

      def data_decl(d)
        @local_types << d["name"]
        selectors = []
        cons = d["cons"].map do |c|
          @con_arity[c["name"]] = c["arity"]
          @local_cons << c["name"]
          selectors |= c["fields"] if c["fields"]
          shown = c["name"].start_with?(":") ? "(#{c['name']})" : c["name"]
          if c["fields"]
            "#{shown} { #{c['fields'].map { |f| "#{f} :: T" }.join(', ')} }"
          else
            ([shown] + Array.new(c["arity"], "t")).join(" ")
          end
        end
        deriving = d["deriving"].empty? ? "" : " deriving (#{d['deriving'].join(', ')})"
        decl = "#{([d['name']] + d['tyvars']).join(' ')} = #{cons.join(' | ')}#{deriving}"
        lines = ["include HaskellMatch.data(#{rb_str(decl)}, under: self, scope: haskell_scope)"]
        # record fields are selector functions, as in Haskell
        selectors.each do |f|
          lines << "define_singleton_method(#{f.to_sym.inspect}) { |v| v.public_send(#{f.to_sym.inspect}) } unless singleton_class.method_defined?(#{f.to_sym.inspect})"
        end
        lines.join("\n")
      end

      def con_arity(name)
        return @con_arity[name] if @con_arity.key?(name)
        return 0 if %w[True False []].include?(name)

        info = Native.constructor_info(name, @scope)
        @con_arity[name] = info && info[1]
      end

      def function(d)
        name = d["name"]
        ivar = @top[name][:ivar]
        scope = Scope.new({}, {}, nil)
        clauses = equations(d["equations"], scope, d["arity"], name)
        <<~RUBY
          #{ivar} = HaskellMatch.fn(#{rb_str(name)}, exhaustive: #{@exhaustive.inspect}, scope: haskell_scope) do |m|
          #{clauses}
          end
          define_singleton_method(#{name.to_sym.inspect}) { |*a| HaskellMatch::Haskell.apply(#{ivar}, #{d['arity']}, a) }
          #{snake(name) == name ? '' : "singleton_class.alias_method(#{snake(name).to_sym.inspect}, #{name.to_sym.inspect})"}
        RUBY
      end

      def value(d)
        name = d["pat"]["vars"][0]
        meth = @values[name]
        scope = Scope.new({}, {}, nil)
        rhs = rhs_code(d["rhs"], d["where"], scope, tail: false, line: d["line"])
        <<~RUBY
          define_singleton_method(#{meth.to_sym.inspect}) { @#{meth} ||= begin; #{rhs}; end }
          define_singleton_method(#{name.to_sym.inspect}) { |*a| a.inject(#{meth}) { |f, x| f.(x) } }
          #{snake(name) == name ? '' : "singleton_class.alias_method(#{snake(name).to_sym.inspect}, #{name.to_sym.inspect})"}
        RUBY
      end

      # Clauses for a list of equations (all of the same arity).
      def equations(eqs, scope, arity, fname, prefix_params: [])
        eqs.map do |eq|
          pats = eq["pats"]
          vars = pats.flat_map { |p| p["vars"] }
          dup = vars.detect { |v| vars.count(v) > 1 }
          raise DuplicateVariableError, "#{@source_name}:#{ln(eq['line'])}: conflicting definitions for '#{dup}' in '#{fname}'" if dup

          inner = Scope.new({}, {}, scope)
          (prefix_params + vars).each { |v| inner.vars[v] = mangle(v) }
          pat_texts = prefix_params.map { |v| rb_str(mangle(v)) } + pats.map { |p| rb_str(p["text"]) }
          params = (prefix_params + vars).map { |v| mangle(v) }
          param_list = params.empty? ? "" : "|#{params.join(', ')}|"
          clauses_for_rhs(eq["rhs"], eq["where"], inner, pat_texts, param_list, ln(eq["line"]))
        end.join("\n")
      end

      # One or more `m.on(...)` lines for an equation's right-hand side.
      def clauses_for_rhs(rhs, wheres, scope, pat_texts, param_list, line)
        where_code = where_bindings(wheres, scope)
        loc = line ? ", location: [#{rb_str(@source_name)}, #{line}]" : ""
        if rhs.key?("body")
          body = expr(rhs["body"], scope, tail: true)
          "  m.on(#{pat_texts.join(', ')}#{loc}) { #{param_list} #{where_code}#{body} }"
        else
          rhs["guards"].map do |(quals, e)|
            if otherwise?(quals)
              body = expr(e, scope, tail: true)
              "  m.on(#{pat_texts.join(', ')}#{loc}) { #{param_list} #{where_code}#{body} }"
            elsif simple_guards?(quals)
              body = expr(e, scope, tail: true)
              guard = quals.map { |q| "(#{expr(q[1], scope, tail: false)})" }.join(" && ")
              "  m.on(#{pat_texts.join(', ')}, guard: ->(#{param_list.delete('|')}) { #{where_code}#{guard} }#{loc}) { #{param_list} #{where_code}#{body} }"
            else
              # pattern guards / let guards: the guard lambda runs the
              # qualifiers for their truth; the body runs them again for
              # their bindings (Haskell code is pure, so this is sound).
              gscope = Scope.new({}, {}, scope)
              guard = qual_chain(quals, gscope) { "true" }
              bscope = Scope.new({}, {}, scope)
              body = qual_chain(quals, bscope) { expr(e, bscope, tail: true) }
              "  m.on(#{pat_texts.join(', ')}, guard: ->(#{param_list.delete('|')}) { #{where_code}#{guard} }#{loc}) { #{param_list} #{where_code}#{body} }"
            end
          end.join("\n")
        end
      end

      # `| otherwise` / `| True`: an unconditional alternative.
      def otherwise?(quals)
        return false unless quals.size == 1 && quals[0][0] == "guard"

        g = quals[0][1]
        (g[0] == "var" && g[1] == "otherwise") || (g[0] == "con" && g[1] == "True")
      end

      def simple_guards?(quals)
        quals.all? { |q| q[0] == "guard" }
      end

      # Qualifiers (boolean guards, `pat <- e` generators binding variables,
      # `let` bindings) as nested Ruby expressions ending in `final`; a
      # failing guard or pattern yields `false`.
      def qual_chain(quals, scope, &final)
        return final.call if quals.empty?

        q, *rest = quals
        case q[0]
        when "guard"
          "((#{expr(q[1], scope, tail: false)}) ? (#{qual_chain(rest, scope, &final)}) : false)"
        when "gen"
          src = expr(q[2], scope, tail: false)
          pat = q[1]
          if pat["vars"].size == 1 && pat["text"] == mangle(pat["vars"][0])
            name = fresh(mangle(pat["vars"][0]))
            scope.vars[pat["vars"][0]] = name
            "(#{name} = #{src}; #{qual_chain(rest, scope, &final)})"
          else
            matcher = "@hs_pat_#{fresh('guard')}"
            @lifted << "#{matcher} = HaskellMatch.pattern(#{rb_str(pat['text'])}, scope: haskell_scope)"
            binds = fresh("hs_b")
            pat["vars"].each { |v| scope.vars[v] = "#{binds}[#{mangle(v).to_sym.inspect}]" }
            "((#{binds} = #{matcher}.match(#{src})) ? (#{qual_chain(rest, scope, &final)}) : false)"
          end
        when "let"
          "(#{where_bindings(q[1], scope)}#{qual_chain(rest, scope, &final)})"
        else
          raise DefinitionError, "#{@source_name}: unknown qualifier #{q[0]}"
        end
      end

      # Guarded alternatives as one expression: `[quals, e]` arms tried in
      # order, `fallback` when none applies.
      def guard_chain(arms, scope, tail, fallback)
        chain = arms.map do |(quals, e)|
          if otherwise?(quals)
            "true ? (#{expr(e, scope, tail: tail)}) : "
          elsif simple_guards?(quals)
            conds = quals.map { |q| "(#{expr(q[1], scope, tail: false)})" }.join(" && ")
            "(#{conds}) ? (#{expr(e, scope, tail: tail)}) : "
          else
            s = Scope.new({}, {}, scope)
            tmp = fresh("hs_g")
            "((#{tmp} = #{qual_chain(quals, s) { "[#{expr(e, s, tail: tail)}]" }})) ? (#{tmp}[0]) : "
          end
        end.join
        "(#{chain}#{fallback})"
      end

      # Right-hand side as a single Ruby expression (for values and lifted
      # case alternatives without their own clauses).
      def rhs_code(rhs, wheres, scope, tail:, line:)
        where_code = where_bindings(wheres, scope)
        if rhs.key?("body")
          "#{where_code}#{expr(rhs['body'], scope, tail: tail)}"
        else
          fallback = "raise(HaskellMatch::Prelude::HaskellError, #{rb_str("#{@source_name}:#{line}: non-exhaustive guards")})"
          "#{where_code}#{guard_chain(rhs['guards'], scope, tail, fallback)}"
        end
      end

      # `where` / `let` declarations: functions are lifted, values become
      # local assignments (strict, in source order).  Returns Ruby code to
      # prepend to the body, and extends `scope`.
      def where_bindings(decls, scope)
        return "" if decls.nil? || decls.empty?

        # register functions first so values and functions can refer to them
        funs = decls.select { |d| d["kind"] == "fun" }
        values = decls.select { |d| d["kind"] == "bind" }
        decls.each do |d|
          next if %w[fun bind sig].include?(d["kind"])

          raise DefinitionError, "#{@source_name}:#{ln(d['line'])}: data declarations are only allowed at top level"
        end
        lift_functions(funs, scope)
        code = +""
        order_values(values).each do |d|
          pat = d["pat"]
          if pat["vars"].size == 1 && pat["text"] == mangle(pat["vars"][0])
            name = pat["vars"][0]
            local = fresh(mangle(name))
            rhs = rhs_code(d["rhs"], d["where"], scope, tail: false, line: d["line"])
            scope.vars[name] = local
            code << "#{local} = (#{rhs}); "
          else
            # pattern binding: destructure through a single-clause match
            tmp = fresh("hs_pb")
            rhs = rhs_code(d["rhs"], d["where"], scope, tail: false, line: d["line"])
            code << "#{tmp} = HaskellMatch.pattern(#{rb_str(pat['text'])}, scope: haskell_scope).match!(#{rhs}); "
            pat["vars"].each do |v|
              local = fresh(mangle(v))
              scope.vars[v] = local
              code << "#{local} = #{tmp}[#{mangle(v).to_sym.inspect}]; "
            end
          end
        end
        code
      end

      # Haskell's `where`/`let` values may refer to one another in any order
      # (they are lazy); Ruby locals are strict, so emit each value after the
      # sibling values it mentions.  A genuine cycle keeps declaration order.
      def order_values(values)
        names = values.flat_map { |d| d["pat"]["vars"] }
        deps = values.map do |d|
          refs = []
          collect_refs(d["rhs"], d["where"], [], refs)
          (refs & names) - d["pat"]["vars"]
        end
        done = []
        out = []
        remaining = values.each_index.to_a
        until remaining.empty?
          i = remaining.find { |j| (deps[j] - done).empty? } || remaining.first
          remaining.delete(i)
          out << values[i]
          done.concat(values[i]["pat"]["vars"])
        end
        out
      end

      # Lambda-lift a group of local functions (mutually recursive allowed).
      def lift_functions(funs, scope)
        return if funs.empty?

        names = funs.map { |d| d["name"] }
        # free variables of each function, excluding its own params and
        # sibling names; then close over siblings' free variables
        free = {}
        funs.each do |d|
          fv = []
          d["equations"].each do |eq|
            bound = eq["pats"].flat_map { |p| p["vars"] }
            collect_free(eq["rhs"], eq["where"], bound + names, scope, fv)
          end
          free[d["name"]] = fv.uniq
        end
        loop do
          changed = false
          funs.each do |d|
            refs = []
            d["equations"].each do |eq|
              bound = eq["pats"].flat_map { |p| p["vars"] }
              collect_refs(eq["rhs"], eq["where"], bound, refs)
            end
            refs.each do |r|
              next unless names.include?(r) && r != d["name"]

              merged = (free[d["name"]] + free[r]).uniq
              if merged != free[d["name"]]
                free[d["name"]] = merged
                changed = true
              end
            end
          end
          break unless changed
        end
        funs.each do |d|
          ivar = "@hs_lift_#{fresh(rb_ident(d['name']))}"
          scope.funs[d["name"]] = { ivar: ivar, arity: d["arity"], free: free[d["name"]] }
        end
        funs.each do |d|
          info = scope.funs[d["name"]]
          @lifted_lines[d["name"]] ||= ln(d["equations"].first&.fetch("line", nil))
          inner = Scope.new({}, {}, scope)
          clauses = equations(d["equations"], inner, d["arity"], d["name"], prefix_params: info[:free])
          @lifted << <<~RUBY
            #{info[:ivar]} = HaskellMatch.fn(#{rb_str(d['name'])}, exhaustive: #{@exhaustive.inspect}, scope: haskell_scope) do |m|
            #{clauses}
            end
          RUBY
        end
      end

      # ------------------------------------------------------- free variables

      # Haskell variables referenced in an rhs that are bound in `scope` as
      # local vars/funs (i.e. would need to be passed to a lifted function).
      def collect_free(rhs, wheres, bound, scope, out)
        where_bound = (wheres || []).flat_map { |d| d["kind"] == "fun" ? [d["name"]] : d["kind"] == "bind" ? d["pat"]["vars"] : [] }
        if rhs.key?("body")
          free_in(rhs["body"], bound + where_bound, scope, out)
        else
          rhs["guards"].each { |(quals, e)| free_in_quals(quals, e, bound + where_bound, scope, out) }
        end
        (wheres || []).each do |d|
          case d["kind"]
          when "fun"
            d["equations"].each do |eq|
              collect_free(eq["rhs"], eq["where"], bound + where_bound + eq["pats"].flat_map { |p| p["vars"] }, scope, out)
            end
          when "bind"
            collect_free(d["rhs"], d["where"], bound + where_bound, scope, out)
          end
        end
      end

      # Free variables of qualifiers followed by `body`; generator patterns
      # and let bindings bind names for what follows them.
      def free_in_quals(quals, body, bound, scope, out)
        b = bound.dup
        quals.each do |q|
          case q[0]
          when "guard" then free_in(q[1], b, scope, out)
          when "gen"
            free_in(q[2], b, scope, out)
            b += q[1]["vars"]
          when "let"
            names = q[1].flat_map { |d| d["kind"] == "fun" ? [d["name"]] : d["kind"] == "bind" ? d["pat"]["vars"] : [] }
            q[1].each do |d|
              case d["kind"]
              when "fun"
                d["equations"].each do |eq|
                  collect_free(eq["rhs"], eq["where"], b + names + eq["pats"].flat_map { |p| p["vars"] }, scope, out)
                end
              when "bind" then collect_free(d["rhs"], d["where"], b + names, scope, out)
              end
            end
            b += names
          end
        end
        free_in(body, b, scope, out)
      end

      def free_in(e, bound, scope, out)
        case e[0]
        when "var"
          name = e[1]
          return if bound.include?(name)

          if scope.lookup_var(name)
            out << name unless out.include?(name)
          elsif (f = scope.lookup_fun(name))
            f[:free].each { |v| out << v unless out.include?(v) || bound.include?(v) }
          end
        when "con", "lit" then nil
        when "opfun"
          free_in(["var", e[1]], bound, scope, out) if user_function?(e[1], scope) || scope.lookup_var(e[1])
        when "multiif"
          e[1].each { |(quals, body)| free_in_quals(quals, body, bound, scope, out) }
        when "reccon" then e[2].each { |(_, v)| free_in(v, bound, scope, out) }
        when "recupd"
          free_in(e[1], bound, scope, out)
          e[2].each { |(_, v)| free_in(v, bound, scope, out) }
        when "app"
          free_in(e[1], bound, scope, out)
          e[2].each { |a| free_in(a, bound, scope, out) }
        when "op"
          free_in(["var", e[1]], bound, scope, out) if user_function?(e[1], scope)
          free_in(e[2], bound, scope, out)
          free_in(e[3], bound, scope, out)
        when "neg" then free_in(e[1], bound, scope, out)
        when "if" then e[1..3].each { |x| free_in(x, bound, scope, out) }
        when "case"
          free_in(e[1], bound, scope, out)
          e[2].each do |alt|
            collect_free(alt["rhs"], alt["where"], bound + alt["pat"]["vars"], scope, out)
          end
        when "let"
          inner = bound + e[1].flat_map { |d| d["kind"] == "fun" ? [d["name"]] : d["kind"] == "bind" ? d["pat"]["vars"] : [] }
          e[1].each do |d|
            case d["kind"]
            when "fun" then d["equations"].each { |eq| collect_free(eq["rhs"], eq["where"], inner + eq["pats"].flat_map { |p| p["vars"] }, scope, out) }
            when "bind" then collect_free(d["rhs"], d["where"], inner, scope, out)
            end
          end
          free_in(e[2], inner, scope, out)
        when "lam"
          free_in(e[2], bound + e[1].flat_map { |p| p["vars"] }, scope, out)
        when "list", "tuple" then e[1].each { |x| free_in(x, bound, scope, out) }
        when "range"
          free_in(e[1], bound, scope, out)
          free_in(e[2], bound, scope, out) if e[2]
          free_in(e[3], bound, scope, out) if e[3]
        when "comp"
          inner = bound.dup
          e[2].each do |q|
            case q[0]
            when "gen"
              free_in(q[2], inner, scope, out)
              inner += q[1]["vars"]
            when "guard" then free_in(q[1], inner, scope, out)
            when "let"
              inner += q[1].flat_map { |d| d["kind"] == "bind" ? d["pat"]["vars"] : [d["name"]] }
              q[1].each { |d| collect_free(d["rhs"], d["where"], inner, scope, out) if d["kind"] == "bind" }
            end
          end
          free_in(e[1], inner, scope, out)
        when "section_l", "section_r" then free_in(e[2], bound, scope, out)
        end
      end

      # All variable names referenced (for sibling closure), ignoring scope.
      def collect_refs(rhs, wheres, bound, out)
        if rhs.key?("body")
          refs_in(rhs["body"], out)
        else
          rhs["guards"].each { |(quals, e)| refs_in_quals(quals, e, bound, out) }
        end
        (wheres || []).each do |d|
          case d["kind"]
          when "fun" then d["equations"].each { |eq| collect_refs(eq["rhs"], eq["where"], bound, out) }
          when "bind" then collect_refs(d["rhs"], d["where"], bound, out)
          end
        end
      end

      def refs_in_quals(quals, body, bound, out)
        quals.each do |q|
          case q[0]
          when "guard" then refs_in(q[1], out)
          when "gen" then refs_in(q[2], out)
          when "let" then collect_refs({ "body" => ["lit", ["int", "0"]] }, q[1], bound, out)
          end
        end
        refs_in(body, out)
      end

      def refs_in(e, out)
        case e[0]
        when "var" then out << e[1]
        when "op"
          out << e[1]
          [e[2], e[3]].each { |x| refs_in(x, out) }
        when "opfun" then out << e[1]
        when "multiif" then e[1].each { |(quals, body)| refs_in_quals(quals, body, [], out) }
        when "reccon" then e[2].each { |(_, v)| refs_in(v, out) }
        when "recupd"
          refs_in(e[1], out)
          e[2].each { |(_, v)| refs_in(v, out) }
        when "app" then (e[2] + [e[1]]).each { |x| refs_in(x, out) }
        when "neg", "section_l", "section_r" then refs_in(e[-1], out)
        when "if", "list", "tuple" then e[1..].flatten(1).each { |x| refs_in(x, out) if x.is_a?(Array) && x[0].is_a?(String) }
        when "case"
          refs_in(e[1], out)
          e[2].each { |alt| collect_refs(alt["rhs"], alt["where"], [], out) }
        when "let"
          e[1].each { |d| collect_refs(d["rhs"], d["where"], [], out) if d.key?("rhs") }
          e[1].each { |d| d["equations"].each { |eq| collect_refs(eq["rhs"], eq["where"], [], out) } if d["kind"] == "fun" }
          refs_in(e[2], out)
        when "lam" then refs_in(e[2], out)
        when "range" then e[1..3].compact.each { |x| refs_in(x, out) }
        when "comp"
          refs_in(e[1], out)
          e[2].each { |q| q[1..].each { |x| refs_in(x, out) if x.is_a?(Array) && x[0].is_a?(String) } }
        end
      end

      # ------------------------------------------------------------ expressions

      # `tail`: this expression's value is the enclosing clause body's value.
      def expr(e, scope, tail:)
        case e[0]
        when "var" then var(e[1], scope)
        when "con" then con_value(e[1])
        when "lit" then literal(e[1])
        when "app" then application(e[1], e[2], scope, tail)
        when "op" then binop(e[1], e[2], e[3], scope, tail)
        when "neg" then "(-(#{expr(e[1], scope, tail: false)}))"
        when "if"
          "((#{expr(e[1], scope, tail: false)}) ? (#{expr(e[2], scope, tail: tail)}) : (#{expr(e[3], scope, tail: tail)}))"
        when "case" then case_expr(e[1], e[2], scope, tail)
        when "let" then let_expr(e[1], e[2], scope, tail)
        when "lam" then lambda_expr(e[1], e[2], scope)
        when "list" then "[#{e[1].map { |x| expr(x, scope, tail: false) }.join(', ')}]"
        when "tuple" then "[#{e[1].map { |x| expr(x, scope, tail: false) }.join(', ')}]"
        when "range"
          args = [expr(e[1], scope, tail: false)]
          args << (e[2] ? expr(e[2], scope, tail: false) : "nil")
          args << expr(e[3], scope, tail: false) if e[3]
          "HaskellMatch::Prelude.range(#{args.join(', ')})"
        when "comp" then comprehension(e[1], e[2], scope)
        when "multiif"
          fallback = "raise(HaskellMatch::Prelude::HaskellError, #{rb_str("#{@source_name}: non-exhaustive guards in multi-way if")})"
          guard_chain(e[1], scope, tail, fallback)
        when "reccon"
          name = e[1]
          raise UnknownConstructorError, "#{@source_name}: not in scope: data constructor '#{name}'" if con_arity(name).nil?

          fields = e[2].map { |(f, v)| "#{f.to_sym.inspect} => #{expr(v, scope, tail: false)}" }
          "#{con_path(name)}.new(#{fields.join(', ')})"
        when "recupd"
          fields = e[2].map { |(f, v)| "#{f.to_sym.inspect} => #{expr(v, scope, tail: false)}" }
          "#{expr(e[1], scope, tail: false)}.with(#{fields.join(', ')})"
        when "section_l"
          # (e op) = \y -> e op y
          y = fresh("hs_sec")
          inner = Scope.new({ "__sec" => y }, {}, scope)
          "->(#{y}) { #{binop(e[1], e[2], ['var', '__sec'], inner, false)} }"
        when "section_r"
          x = fresh("hs_sec")
          inner = Scope.new({ "__sec" => x }, {}, scope)
          "->(#{x}) { #{binop(e[1], ['var', '__sec'], e[2], inner, false)} }"
        when "opfun" then op_value(e[1], scope)
        else raise DefinitionError, "#{@source_name}: unknown expression node #{e[0]}"
        end
      end

      def literal(l)
        case l[0]
        when "int" then l[1]
        when "float" then l[1]
        when "char", "str" then rb_str(l[1])
        end
      end

      # A variable used as a value.
      def var(name, scope)
        if (local = scope.lookup_var(name))
          return local
        end
        if (f = scope.lookup_fun(name))
          return curried_call(f[:ivar], f[:free].map { |v| scope.lookup_var(v) || mangle(v) }, f[:arity] + f[:free].size)
        end
        if @top.key?(name)
          info = @top[name]
          return info[:arity] == 1 ? "#{info[:ivar]}.to_proc" : "#{info[:ivar]}.curried"
        end
        return @values[name] if @values.key?(name)
        if Prelude.known?(name.to_sym)
          return Prelude.arity(name.to_sym).zero? ? "HaskellMatch::Prelude.#{name}" : "HaskellMatch::Prelude.curried(#{name.to_sym.inspect})"
        end

        # foreign: a Ruby method of the host module
        "method(#{name.to_sym.inspect}).to_proc"
      end

      def curried_call(ivar, pre_args, total_arity)
        base = total_arity == 1 ? "#{ivar}.to_proc" : "#{ivar}.curried"
        pre_args.inject(base) { |acc, a| "#{acc}.(#{a})" }
      end

      def con_value(name)
        arity = con_arity(name)
        raise UnknownConstructorError, "#{@source_name}: not in scope: data constructor '#{name}'" if arity.nil?

        path = con_path(name)
        return path if arity.zero?

        arity == 1 ? "#{path}.to_proc" : "#{path}.method(:new).to_proc.curry(#{arity})"
      end

      # Constructors declared by this module are constants of the host module;
      # anything else (Prelude types, `HaskellMatch.data` declared elsewhere)
      # is referenced by the permanent class path of its registration.
      def con_path(name)
        return name.downcase if %w[True False].include?(name)
        return "[]" if name == "[]"
        return HaskellMatch.constructor_constant(name) if @con_arity.key?(name) && @local_cons.include?(name)

        reg = HaskellMatch.constructor(name, @scope)
        raise UnknownConstructorError, "#{@source_name}: not in scope: data constructor '#{name}'" if reg.nil?

        (reg.is_a?(Class) ? reg : reg.class).name
      end

      # `(op)` as a value: a constructor, a user-defined function or
      # operator, or a Prelude operator.
      def op_value(op, scope)
        return con_value(op) if op.start_with?(":") && op != ":"
        return var(op, scope) if user_function?(op, scope) || !symbolic?(op)

        "HaskellMatch::Prelude.op(#{rb_str(op)})"
      end

      def application(f, args, scope, tail)
        arg_code = args.map { |a| expr(a, scope, tail: false) }
        if f[0] == "var"
          name = f[1]
          if scope.lookup_var(name).nil?
            if (lf = scope.lookup_fun(name))
              pre = lf[:free].map { |v| scope.lookup_var(v) || mangle(v) }
              return known_call(lf[:ivar], lf[:arity], pre + arg_code, tail, pre.size)
            end
            if @top.key?(name)
              info = @top[name]
              return known_call(info[:ivar], info[:arity], arg_code, tail, 0)
            end
            if !@values.key?(name) && Prelude.known?(name.to_sym)
              arity = Prelude.arity(name.to_sym)
              if arity.positive? && arg_code.size >= arity
                direct = "HaskellMatch::Prelude.#{name}(#{arg_code.first(arity).join(', ')})"
                return apply_rest(direct, arg_code.drop(arity))
              end
              return apply_rest("HaskellMatch::Prelude.curried(#{name.to_sym.inspect})", arg_code)
            end
            if !@values.key?(name) && !scope.shadowed?(name) && !Prelude.known?(name.to_sym)
              # foreign Ruby method on the host module
              return "__send__(#{name.to_sym.inspect}, #{arg_code.join(', ')})" if symbolic?(name)

              return "#{name}(#{arg_code.join(', ')})"
            end
          end
        elsif f[0] == "con"
          name = f[1]
          arity = con_arity(name)
          raise UnknownConstructorError, "#{@source_name}: not in scope: data constructor '#{name}'" if arity.nil?

          if arg_code.size >= arity && arity.positive?
            return apply_rest("#{con_path(name)}.new(#{arg_code.first(arity).join(', ')})", arg_code.drop(arity))
          end
        end
        apply_rest(expr(f, scope, tail: false), arg_code)
      end

      # Call a Function held in `ivar` with `arity` own parameters (plus
      # `pre` leading free-variable arguments already included in `args`).
      def known_call(ivar, arity, args, tail, pre)
        total = arity + pre
        if args.size == total
          tail ? "#{ivar}.tail(#{args.join(', ')})" : "#{ivar}.(#{args.join(', ')})"
        elsif args.size > total
          apply_rest("#{ivar}.(#{args.first(total).join(', ')})", args.drop(total))
        else
          curried_call(ivar, args, total)
        end
      end

      # An expression whose evaluation is immediate and cannot recurse.
      def trivial?(e)
        case e[0]
        when "var", "lit", "con" then true
        when "list", "tuple" then e[1].all? { |x| trivial?(x) }
        when "neg" then trivial?(e[1])
        else false
        end
      end

      def apply_rest(code, rest)
        rest.inject(code) { |acc, a| "#{acc}.(#{a})" }
      end

      def binop(op, l, r, scope, tail)
        # an infix constructor builds a value; a user-defined operator or a
        # backticked function is an ordinary call
        return application(["con", op], [l, r], scope, tail) if op.start_with?(":") && op != ":"
        return application(["var", op], [l, r], scope, tail) if user_function?(op, scope)
        return application(["var", op], [l, r], scope, tail) if !symbolic?(op) && op != "seq"

        if op == "$"
          # f $ x  ==  f x
          return application(l, [r], scope, tail) if l[0] == "var" || l[0] == "con"
          return application(l[1], l[2] + [r], scope, tail) if l[0] == "app"

          return "#{expr(l, scope, tail: false)}.(#{expr(r, scope, tail: false)})"
        end
        if op == "seq"
          return "(#{expr(l, scope, tail: false)}; #{expr(r, scope, tail: tail)})"
        end
        if op == ":" && !trivial?(r)
          # Haskell's `:` does not evaluate its tail; a tail that is a
          # computation (not a variable or literal) becomes a thunk so that
          # `p : sieve xs` and `fibs = 0 : 1 : zipWith (+) fibs (tail fibs)`
          # build infinite lists instead of looping.
          return "HaskellMatch::LazyList.lazy_cons(#{expr(l, scope, tail: false)}) { #{expr(r, scope, tail: false)} }"
        end
        binop_code(op, expr(l, scope, tail: false), expr(r, scope, tail: false))
      end

      def binop_code(op, lc, rc)
        if BINOPS.key?(op)
          "(#{lc} #{BINOPS[op]} #{rc})"
        elsif PRELUDE_BINOPS.key?(op)
          "HaskellMatch::Prelude.#{PRELUDE_BINOPS[op]}(#{lc}, #{rc})"
        elsif op == "$"
          "#{lc}.(#{rc})"
        elsif Prelude.known?(op.to_sym) && Prelude.arity(op.to_sym) == 2
          "HaskellMatch::Prelude.#{op}(#{lc}, #{rc})"
        else
          raise DefinitionError, "#{@source_name}: operator '#{op}' is not supported"
        end
      end

      # `case` is lifted into a function whose arguments are the free
      # variables of the alternatives followed by the scrutinee.
      def case_expr(scrut, alts, scope, tail)
        free = []
        alts.each { |alt| collect_free(alt["rhs"], alt["where"], alt["pat"]["vars"], scope, free) }
        ivar = "@hs_lift_#{fresh('case')}"
        inner_scope = Scope.new({}, {}, scope)
        clauses = alts.map do |alt|
          vars = alt["pat"]["vars"]
          dup = (free + vars).detect { |v| (free + vars).count(v) > 1 }
          if dup
            # a pattern variable shadowing a captured one: rename the capture
            raise DefinitionError, "#{@source_name}:#{ln(alt['line'])}: '#{dup}' is both captured and bound in a case alternative; rename one"
          end
          s = Scope.new({}, {}, inner_scope)
          (free + vars).each { |v| s.vars[v] = mangle(v) }
          params = (free + vars).map { |v| mangle(v) }
          param_list = params.empty? ? "" : "|#{params.join(', ')}|"
          pat_texts = free.map { |v| rb_str(mangle(v)) } + [rb_str(alt["pat"]["text"])]
          clauses_for_rhs(alt["rhs"], alt["where"], s, pat_texts, param_list, ln(alt["line"]))
        end.join("\n")
        @lifted << <<~RUBY
          #{ivar} = HaskellMatch.fn("case", exhaustive: #{@exhaustive.inspect}, scope: haskell_scope) do |m|
          #{clauses}
          end
        RUBY
        args = free.map { |v| scope.lookup_var(v) || mangle(v) } + [expr(scrut, scope, tail: false)]
        tail ? "#{ivar}.tail(#{args.join(', ')})" : "#{ivar}.(#{args.join(', ')})"
      end

      def let_expr(decls, body, scope, tail)
        inner = Scope.new({}, {}, scope)
        code = where_bindings(decls, inner)
        "(#{code}#{expr(body, inner, tail: tail)})"
      end

      # Lambdas with only variable patterns are curried Ruby lambdas; others
      # are lifted into a function and partially applied to their free vars.
      def lambda_expr(pats, body, scope)
        if pats.all? { |p| p["vars"].size == 1 && p["text"] == mangle(p["vars"][0]) }
          inner = Scope.new({}, {}, scope)
          names = pats.map { |p| fresh(mangle(p["vars"][0])) }
          pats.each_with_index { |p, i| inner.vars[p["vars"][0]] = names[i] }
          b = expr(body, inner, tail: false)
          return names.reverse.inject(b) { |acc, n| "->(#{n}) { #{acc} }" }
        end
        free = []
        bound = pats.flat_map { |p| p["vars"] }
        free_in(body, bound, scope, free)
        ivar = "@hs_lift_#{fresh('lambda')}"
        s = Scope.new({}, {}, scope)
        (free + bound).each { |v| s.vars[v] = mangle(v) }
        params = (free + bound).map { |v| mangle(v) }
        pat_texts = free.map { |v| rb_str(mangle(v)) } + pats.map { |p| rb_str(p["text"]) }
        @lifted << <<~RUBY
          #{ivar} = HaskellMatch.fn("lambda", exhaustive: #{@exhaustive.inspect}, scope: haskell_scope) do |m|
            m.on(#{pat_texts.join(', ')}) { |#{params.join(', ')}| #{expr(body, s, tail: true)} }
          end
        RUBY
        curried_call(ivar, free.map { |v| scope.lookup_var(v) || mangle(v) }, free.size + pats.size)
      end

      def comprehension(body, quals, scope)
        inner = Scope.new({}, {}, scope)
        code = comp_quals(quals, body, inner)
        "(HaskellMatch::Prelude.comp_begin; HaskellMatch::Prelude.comp_finish(#{code}))"
      end

      def comp_quals(quals, body, scope)
        if quals.empty?
          return "[#{expr(body, scope, tail: false)}].lazy"
        end
        q, *rest = quals
        case q[0]
        when "gen"
          src = expr(q[2], scope, tail: false)
          pat = q[1]
          if pat["vars"].size == 1 && pat["text"] == mangle(pat["vars"][0])
            name = fresh(mangle(pat["vars"][0]))
            scope.vars[pat["vars"][0]] = name
            "HaskellMatch::Prelude.gen(#{src}).flat_map { |#{name}| #{comp_quals(rest, body, scope)} }"
          else
            # refutable pattern: elements that do not match are skipped
            matcher = "@hs_pat_#{fresh('comp')}"
            @lifted << "#{matcher} = HaskellMatch.pattern(#{rb_str(pat['text'])}, scope: haskell_scope)"
            el = fresh("hs_el")
            binds = fresh("hs_b")
            pat["vars"].each { |v| scope.vars[v] = "#{binds}[#{mangle(v).to_sym.inspect}]" }
            "HaskellMatch::Prelude.gen(#{src}).flat_map { |#{el}| (#{binds} = #{matcher}.match(#{el})) ? (#{comp_quals(rest, body, scope)}) : [].lazy }"
          end
        when "guard"
          "((#{expr(q[1], scope, tail: false)}) ? (#{comp_quals(rest, body, scope)}) : [].lazy)"
        when "let"
          code = where_bindings(q[1], scope)
          "(#{code}#{comp_quals(rest, body, scope)})"
        end
      end
    end
  end
end
