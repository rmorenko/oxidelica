model R3
  function readLine
    input String fileName; input Integer lineNumber(min=1);
    output String string; output Boolean endOfFile;
    external "C" string = ModelicaInternal_readLine(fileName, lineNumber, endOfFile);
  end readLine;
  function cscan
    input String string; input Integer startIndex(min=1)=1; input Boolean unsigned=false;
    output Integer nextIndex; output Integer number;
    external "C" ModelicaStrings_scanInteger(string, startIndex, unsigned, nextIndex, number);
  end cscan;
  function skipWhiteSpace
    input String string; input Integer startIndex(min=1)=1; output Integer nextIndex;
    external "C" nextIndex = ModelicaStrings_skipWhiteSpace(string, startIndex);
  end skipWhiteSpace;
  function length
    input String string; output Integer result;
    external "C" result = ModelicaStrings_length(string);
  end length;
  function substring
    input String string; input Integer startIndex(min=1); input Integer endIndex;
    output String result;
    external "C" result = ModelicaStrings_substring(string, startIndex, endIndex);
  end substring;
  function repeat
    input Integer n(min=0) = 1; input String string=" "; output String repeatedString;
  algorithm
    repeatedString := "";
    for i in 1:n loop
      repeatedString := repeatedString + string;
    end for;
  end repeat;
  function error
    input String string;
  algorithm
    assert(false, string, AssertionLevel.error);
  end error;
  function syntaxError
    input String string; input Integer index; input String message="";
  protected
    Integer maxIndex = 40;
    Integer maxLenString = 60;
    Integer lenString = length(string);
    String errString;
    Integer index2 = if index < 1 then 1 else if index > lenString then lenString else index;
  algorithm
    if index2 <= maxIndex then
      errString := string;
    else
      errString := "... " + substring(string, index2-maxIndex, lenString);
      index2 := maxIndex + 5;
    end if;
    if length(errString) > maxLenString then
      errString := substring(errString, 1, maxLenString) + " ...";
    end if;
    error("Syntax error at character " + String(index) + " of\n" +
          errString + "\n" + repeat(index2-1, " ") + "*" + "\n" + message);
  end syntaxError;
  function scanInteger
    input String string; input Integer startIndex(min=1)=1;
    input Boolean unsigned=false; input String message="";
    output Integer number; output Integer nextIndex;
  algorithm
    (nextIndex, number) := cscan(string, startIndex, unsigned);
    if nextIndex == startIndex then
      nextIndex := skipWhiteSpace(string, startIndex);
      if unsigned then
        syntaxError(string, nextIndex, "Expected an Integer number without a sign " + message);
      else
        syntaxError(string, nextIndex, "Expected an Integer number " + message);
      end if;
    end if;
  end scanInteger;
  function g
    input String f;
    output Integer m[2];
  protected
    String data;
  algorithm
    data := readLine(f, 1);
    for i in 1:1 loop
      for j in 1:2 loop m[j] := scanInteger(data, 2*j - 1); end for;
    end for;
  end g;
  parameter String fileName = "DATA";
  Integer m[2](start={0,0});
algorithm
  if initial() then m := g(fileName); end if;
end R3;
