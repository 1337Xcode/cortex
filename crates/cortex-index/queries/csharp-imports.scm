; Each namespace or type a C# file imports: `using System.Text;`, `global using Shop.Core;`, and
; `using static System.Math;`, whose type's static members the file then uses by name. An alias
; binds a name: `using Json = System.Text.Json.JsonSerializer;`.
(using_directive
  !name
  [(identifier) (qualified_name)] @import.module) @import
(using_directive
  name: (identifier) @import.alias
  [(identifier) (qualified_name)] @import.module) @import
