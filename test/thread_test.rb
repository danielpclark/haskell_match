# frozen_string_literal: true

require_relative "test_helper"

class ThreadTest < Minitest::Test
  include TestHelpers

  def test_concurrent_matching_and_definition
    classify = fn(:classify) do
      on("Just (x:_)", guard: ->(x) { x.odd? }) { :odd_head }
      on("Just _") { :just }
      on("Nothing") { :nothing }
    end
    values = [Just.new([1]), Just.new([2]), Just.new([]), Nothing]
    expected = %i[odd_head just just nothing]
    threads = 8.times.map do |t|
      Thread.new do
        200.times do |i|
          assert_equal expected, values.map { |v| classify.(v) }
          HaskellMatch.data("Thr#{t}_#{i % 3} = T#{t}_#{i % 3} Int") if i % 50 == 0
          f = fn { on("Just x") { |x| x }; on("Nothing") { 0 } }
          assert_equal 1, f.(Just.new(1))
        end
      end
    end
    threads.each(&:join)
  end
end
