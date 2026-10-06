-- A plain Haskell module, loaded by `HaskellMatch.require "geometry"` or
-- `HaskellMatch.load("examples/geometry.hs")`.  No interpolation: this is
-- the Haskell 2010 subset haskell_match compiles to Ruby.
module Geometry where

data Shape
  = Circle Double
  | Rect Double Double
  | Triangle Double Double Double

area :: Shape -> Double
area (Circle r) = pi * r * r
area (Rect w h) = w * h
area (Triangle a b c) = sqrt (s * (s - a) * (s - b) * (s - c))
  where s = (a + b + c) / 2

perimeter :: Shape -> Double
perimeter (Circle r) = 2 * pi * r
perimeter (Rect w h) = 2 * (w + h)
perimeter (Triangle a b c) = a + b + c

describe :: Shape -> String
describe s
  | a > 100 = "large " ++ kind
  | a > 10 = "medium " ++ kind
  | otherwise = "small " ++ kind
  where
    a = area s
    kind = case s of
      Circle _ -> "circle"
      Rect w h | w == h -> "square"
               | otherwise -> "rectangle"
      Triangle _ _ _ -> "triangle"

totalArea :: [Shape] -> Double
totalArea = sum . map area

largest :: [Shape] -> Maybe Shape
largest [] = Nothing
largest (s:ss) = Just (go s ss)
  where
    go best [] = best
    go best (x:xs)
      | area x > area best = go x xs
      | otherwise = go best xs
