# frozen_string_literal: true

require_relative "haskell/compiler"

module HaskellMatch
  # Raised when Haskell source fails to parse or compile; wraps the original
  # error with the source location.
  class HaskellSyntaxError < CompileError; end

  # Haskell source compiled into a Ruby module.
  #
  #   module Geometry
  #     extend HaskellMatch::Haskell
  #     haskell <<~HS
  #       data Shape = Circle Double | Rect Double Double
  #       area (Circle r) = pi * r * r
  #       area (Rect w h) = w * h
  #     HS
  #   end
  #   Geometry.area(Geometry::Circle.new(1.0))
  #
  # Or from a file: `HaskellMatch.load("geometry.hs")` returns a module;
  # `HaskellMatch.require "geometry"` finds `geometry.hs` on `$LOAD_PATH` and
  # defines a constant named after the module header (or the file name).
  module Haskell
    # Compile Haskell source into the extending module.  Errors are reported
    # against `file`/`line` (default: the caller, so a heredoc's lines map to
    # the Ruby file).
    def haskell(source, file: nil, line: nil, exhaustive: HaskellMatch.exhaustive)
      file, line = HaskellMatch::Haskell.caller_site(caller_locations(1, 1)) if file.nil?
      HaskellMatch::Haskell.compile(source, self, file: file, line: line, exhaustive: exhaustive)
      self
    end

    # The type scope this module's `data` declarations live in and its
    # patterns compile against: a snapshot of the global registry taken when
    # the module was first compiled, plus the module's own types.
    def haskell_scope
      @__haskell_scope__ ||= HaskellMatch.new_scope
    end

    # `HaskellMatch.fn`, `.case_of`, `.pattern` and `.data` in this module's
    # type scope, so Ruby code can match on the module's Haskell types:
    #
    #   Shapes.fn(:name) { on("Circle _") { "circle" }; on("Rect _ _") { "rect" } }
    def fn(name = nil, **options, &definition)
      HaskellMatch.fn(name, scope: haskell_scope, **options, &definition)
    end

    def case_of(*values, **options, &definition)
      HaskellMatch.case_of(*values, scope: haskell_scope, **options, &definition)
    end

    def pattern(source = nil, &block)
      HaskellMatch.pattern(source, scope: haskell_scope, &block)
    end

    def data(decl, under: self, **constructors)
      HaskellMatch.data(decl, under: under, scope: haskell_scope, **constructors)
    end

    # The {Function} objects behind the module's Haskell functions, by
    # Haskell name (`mod.haskell_functions["insert"].tail(...)`).
    def haskell_functions
      (@__haskell_functions__ ||= {}).dup
    end

    # One compiled function, as a {Function}.
    def haskell_function(name)
      (@__haskell_functions__ ||= {}).fetch(name.to_s) do
        raise NameError, "no Haskell function #{name} in #{self}"
      end
    end

    class << self
      # `file`/`line` locate the source for error messages; `line` is the line
      # before the Haskell text (a heredoc opener), so Haskell line 1 is
      # reported as `line + 1`.
      def compile(source, host, file: "(haskell)", line: nil, exhaustive: HaskellMatch.exhaustive, ast: nil)
        offset = line || 0
        ast ||= parse(source, file, offset)
        host.extend(Haskell) unless host.singleton_class.include?(Haskell)
        compiler = Compiler.new(ast, host, source_name: file, exhaustive: exhaustive, line_offset: offset,
                                           scope: host.haskell_scope)
        ruby = compiler.generate
        begin
          host.module_eval(ruby, "#{file} (compiled)", 1)
        rescue CompileError => e
          name = e.message[/In an equation for '([^']+)'/, 1]
          line = name && (compiler.lines[name] || compiler.lifted_lines[name])
          raise e.exception("#{file}#{line ? ":#{line}" : ''}: #{e.message}"), cause: nil
        end
        host.instance_variable_set(:@__haskell_source__, (host.instance_variable_get(:@__haskell_source__) || []) << source)
        host.instance_variable_set(:@__haskell_ruby__, (host.instance_variable_get(:@__haskell_ruby__) || []) << ruby)
        ast["name"]
      end

      # Parse Haskell source into its JSON AST (a Hash).
      def parse(source, file = "(haskell)", line_offset = 0)
        JSON.parse(Native.parse_haskell(source))
      rescue CompileError => e
        msg = e.message.sub(/\A(\d+):/) { "#{Regexp.last_match(1).to_i + line_offset}:" }
        raise HaskellSyntaxError, "#{file}:#{msg}"
      end

      # [path, line] of the Ruby call site, for default error locations.
      def caller_site(locations)
        loc = locations&.first
        loc ? [loc.path, loc.lineno] : ["(haskell)", 0]
      end

      # The Ruby generated for a module's Haskell (for debugging).
      def generated_ruby(host)
        (host.instance_variable_get(:@__haskell_ruby__) || []).join("\n")
      end

      # Constant path for a loaded file: its `module` header (`Data.Tree` ->
      # `Data::Tree`) or its camelised file name.
      def module_name_for(ast_name, path)
        return ast_name.split(".") if ast_name

        [File.basename(path, ".*").split(/[^A-Za-z0-9]+/).map(&:capitalize).join]
      end

      # Define `mod` as the (possibly nested) constant `names` under `under`.
      def define_constant(under, names, mod)
        *parents, last = names
        home = parents.inject(under) do |ns, n|
          ns.const_defined?(n, false) ? ns.const_get(n, false) : ns.const_set(n, Module.new)
        end
        home.send(:remove_const, last) if home.const_defined?(last, false)
        home.const_set(last, mod)
      end
    end
  end

  class << self
    # Compile Haskell source into a fresh (or given) module.
    def haskell(source, into: Module.new, file: nil, line: nil, exhaustive: self.exhaustive)
      file, line = Haskell.caller_site(caller_locations(1, 1)) if file.nil?
      into.extend(Haskell) unless into.singleton_class.include?(Haskell)
      Haskell.compile(source, into, file: file, line: line, exhaustive: exhaustive)
      into
    end

    # Load a `.hs` file into a module (a new anonymous one by default).
    def load(path, into: Module.new, exhaustive: self.exhaustive)
      source = File.read(path)
      into.extend(Haskell) unless into.singleton_class.include?(Haskell)
      Haskell.compile(source, into, file: path, exhaustive: exhaustive)
      into
    end

    # Find `name.hs` (or a path) on `$LOAD_PATH`, compile it once, and define
    # a constant for it named after its `module` header or its file name.
    # Returns the module.
    def require(name, under: Object)
      path = resolve_hs(name)
      @required_hs ||= {}
      return @required_hs[path] if @required_hs.key?(path)

      source = File.read(path)
      ast = Haskell.parse(source, path)
      mod = Module.new
      mod.extend(Haskell)
      Haskell.compile(source, mod, file: path, ast: ast)
      Haskell.define_constant(under, Haskell.module_name_for(ast["name"], path), mod)
      @required_hs[path] = mod
    end

    private

    def resolve_hs(name)
      name = name.to_s
      candidates = [name, "#{name}.hs"]
      candidates.each { |c| return File.expand_path(c) if File.file?(c) }
      $LOAD_PATH.each do |dir|
        candidates.each do |c|
          full = File.join(dir, c)
          return full if File.file?(full)
        end
      end
      raise LoadError, "cannot load Haskell file -- #{name} (looked for #{name}.hs in the current directory and $LOAD_PATH)"
    end
  end
end
