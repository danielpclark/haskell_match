# frozen_string_literal: true

require "haskell_match"

# Runs every ```ruby block of the README introduction (everything before
# "## Installation") in order; `# => value` comments are checked against the
# actual result.  Usage: ruby -Ilib test/support/doctest.rb README.md
src = File.read(ARGV[0])
src = src[0, src.index("## Installation")] if src.include?("## Installation")
blocks = src.scan(/```ruby\n(.*?)```/m).flatten
$failures = 0
def __check(actual, expected, where)
  return if actual.inspect == expected
  puts "#{where}: expected #{expected}, got #{actual.inspect}"
  $failures += 1
end
__binding = binding
blocks.each_with_index do |block, i|
  if block.match?(/^# (HaskellMatch::\w+Error|Patterns not matched)/)
    code = block.lines.reject { |l| l.start_with?("#") }.join
    begin
      __binding.eval(code, "block#{i}")
      puts "block #{i}: expected an error but none was raised"; $failures += 1
    rescue HaskellMatch::CompileError => e
      puts "block #{i}: raises #{e.class} as documented"
    end
    next
  end
  code = block.lines.each_with_index.map do |line, ln|
    if (m = line.match(/\A(.+?)\s+# => (.+)\n?\z/))
      expr, expected = m[1], m[2].strip
      where = "block #{i} line #{ln + 1}"
      expr.strip == "end" ? "end.then { |__r| __check(__r, #{expected.inspect}, #{where.inspect}) }\n" : "__check((#{expr}), #{expected.inspect}, #{where.inspect})\n"
    else
      line
    end
  end.join
  begin
    __binding.eval(code, "block#{i}")
  rescue Exception => e
    puts "block #{i}: #{e.class}: #{e.message.lines.first}"; $failures += 1
  end
end
puts "doc blocks: #{blocks.size}, failures: #{$failures}"
exit($failures.zero? ? 0 : 1)
