# frozen_string_literal: true

module HaskellMatch
  # Deep mode: `call` implemented in Ruby, so that while a recursion is
  # pending no native frame sits on the machine stack. Each level then costs
  # only its Ruby VM frames (a few hundred bytes) and the GC scans only those,
  # which is what makes very deep recursion cheap. The price is one extra
  # Ruby method frame per call (see the README's expert notes).
  #
  # The native `prepare` does the matching, counts the nesting level and
  # returns `[values..., body, depth]`; the generated `call` invokes the body
  # (on a fresh Fiber at segment boundaries), pops the level, and hands any
  # `tail`/`defer` marker to the trampoline below, which has the same
  # semantics as the native `call`.
  module DeepCall
    CONT_CHUNK = 256

    class << self
      # Mirrors HaskellMatch.stack_segment (kept here to avoid a native call
      # per invocation).
      attr_accessor :segment
    end
    self.segment = Native.stack_segment

    # Generate a `call` of exact arity (avoids a splat allocation per call).
    def self.module_for(arity)
      (@modules ||= {})[arity] ||= begin
        params = (1..arity).map { |i| "a#{i}" }.join(", ")
        Module.new.tap do |m|
          m.module_eval(<<~RUBY, __FILE__, __LINE__ + 1)
            def call(#{params})
              vals = prepare(#{params})
              depth = vals.pop
              body = vals.pop
              result = begin
                seg = DeepCall.segment
                if seg > 0 && depth % seg == 0
                  Fiber.new { body.call(*vals) }.resume
                else
                  body.call(*vals)
                end
              ensure
                Native.depth_pop
              end
              TailCall === result ? DeepCall.trampoline(result) : result
            end

            def deep?
              true
            end
          RUBY
        end
      end
    end

    # Run `body` with `vals`, on a fresh Fiber at segment boundaries.
    def self.invoke(body, vals, depth)
      seg = segment
      if seg > 0 && (depth % seg).zero?
        Fiber.new { body.call(*vals) }.resume
      else
        body.call(*vals)
      end
    ensure
      Native.depth_pop
    end

    # Continue a call whose body returned a `TailCall` marker: apply tail
    # calls on the spot and keep `defer` continuations on a chunked stack.
    def self.trampoline(result)
      conts = nil
      while true
        if TailCall === result
          if (k = result.continuation)
            conts = [conts] if conts.nil? || conts.size > CONT_CHUNK
            conts << k
          end
          vals = result.function.prepare(*result.args)
          depth = vals.pop
          body = vals.pop
          result = invoke(body, vals, depth)
        else
          k = nil
          while conts
            if conts.size > 1
              k = conts.pop
              break
            end
            conts = conts[0]
          end
          return result unless k

          result = k.call(result)
        end
      end
    end
  end
end
