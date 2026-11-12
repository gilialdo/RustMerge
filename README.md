# RustMerge

**[Português](#pt) · [English](#en) · [Español](#es)**

---

<a id="pt"></a>

## Português

Comparador de pastas e arquivos em Rust, com interface [egui](https://github.com/emilk/egui).

### O que faz

- Compara duas pastas recursivamente, **em paralelo** (várias threads com `rayon`).
  - Método **Conteúdo**: tamanho e, se for igual, byte a byte.
  - Método **Tamanho e data**: não abre os arquivos; bem mais rápido.
  - Barra de progresso e botão **Cancelar**.
  - **Ignorar arquivos ocultos** (marcado por padrão): pula arquivos e pastas com o atributo
    Oculto do Windows e pastas cujo nome começa com ponto (`.git`, `.vs`, `.idea`...),
    incluindo tudo dentro delas. Arquivos com ponto, como `.gitignore`, continuam aparecendo.
  - **Extensões** (campo logo abaixo das pastas, com menu ⏷ dos filtros já usados): só compara
    arquivos com as extensões informadas, separadas por vírgula, ponto e vírgula ou espaço
    (`rs, .toml *.txt tar.gz`). Com `!` na frente, exclui (`!log`). Vazio = todos. Pastas que
    ficam sem nenhum arquivo são escondidas. `Enter` no campo compara de novo.
- Árvore de arquivos à direita, colorida por status (normal = idêntico, vermelho = diferente,
  azul = só na esquerda, verde = só na direita). A árvore é virtualizada: só desenha as linhas
  visíveis, então aguenta centenas de milhares de arquivos.
- Colunas ao lado do nome: **Situação**, **tamanho** e **data de modificação** de cada lado.
  A largura das colunas se ajusta arrastando as divisórias do cabeçalho e fica salva.
- Ao clicar num arquivo, mostra o diff lado a lado com números de linha, linhas de preenchimento
  alinhando os dois lados e destaque dos caracteres alterados. O arquivo é lido e comparado em
  segundo plano, sem travar a janela.
- Navegação entre diferenças: botões Anterior/Próxima ou `Alt+↑` / `Alt+↓`. `F5` compara de novo.
- **"Editar"** no menu do botão direito de um arquivo abre uma janela de edição com os dois lados
  editáveis, números de linha e as linhas diferentes destacadas (atualiza enquanto você digita).
  Salva cada lado ou os dois (`Ctrl+S`), mantendo a codificação e a quebra de linha (CRLF/LF) do
  arquivo. Se o arquivo só existe de um lado, salvar o outro lado cria o arquivo. Ao fechar com
  alterações não salvas, pergunta se quer salvar. Ao fechar a janela, o arquivo é recomparado e a
  árvore e o diff são atualizados.
- **Botão direito** num arquivo ou pasta: copiar ou mover da esquerda para a direita (ou ao
  contrário), apagar de um lado ou dos dois (vai para a **Lixeira**) e renomear (nos dois lados).
  Pede confirmação antes de substituir, mover ou apagar.
- **Seleção múltipla** na árvore: `Ctrl+clique` marca/desmarca, `Shift+clique` seleciona um
  intervalo. As opções do botão direito valem para todos os selecionados ("Editar" e "Renomear"
  só com um item). Se uma pasta e algo dentro dela estiverem selecionados, a operação vale para
  a pasta. Se algum item falhar, os outros continuam e a barra de status mostra o resumo.
- **Localizar** (`Ctrl+F` ou botão "Localizar"), no diff da tela principal e na janela de edição:
  busca num lado de cada vez (**Esquerda** ou **Direita**, escolhido na barra),
  destaca todas as ocorrências, `Enter`/`F3` vai para a próxima, `Shift+Enter`/`Shift+F3` para a
  anterior, opção de diferenciar maiúsculas, `Esc` fecha. No editor, ao fechar, a ocorrência atual
  fica selecionada.
- Guarda as últimas pastas e filtros de extensão usados (menu ⏷ ao lado de cada campo), o método,
  o número de threads e as opções "Ignorar arquivos ocultos" e "Mostrar só diferenças".
  Ficam em `%APPDATA%\RustMerge\data\app.ron`.
- Detecta UTF-8, UTF-16 (com BOM) e windows-1252; arquivos binários não são exibidos.

### Como rodar

```
cargo run --release
cargo run --release -- C:\pasta\esquerda C:\pasta\direita
```

### Testes

```
cargo test
```

### Desempenho

O exemplo `bench` gera arquivos de teste e mede a comparação com 1, 2, 4... threads:

```
cargo run --release --example bench -- gerar C:\temp\bench 10000
cargo run --release --example bench -- medir C:\temp\bench\esquerda C:\temp\bench\direita
```

Medição numa máquina com 32 núcleos e SSD, 10.000 arquivos por lado (1,4 GB no total), cache aquecido:

| Threads | Tempo | Ganho |
|---|---|---|
| 1 | 0,61 s | 1,0x |
| 2 | 0,35 s | 1,7x |
| 4 | 0,18 s | 3,5x |
| 8 | 0,16 s | 3,9x |
| 16 | 0,21 s | 2,9x |
| 32 | 0,21 s | 2,9x |

Acima de 8 threads o disco e a memória viram o gargalo, por isso o modo automático usa no máximo 8.
Em HD mecânico, 1 ou 2 threads costuma ser melhor.

Outras otimizações:
- Arquivos com tamanhos diferentes são marcados como diferentes sem serem abertos.
- Tamanho e data vêm da própria listagem da pasta (sem consulta extra ao disco no Windows).
- Cada thread reaproveita buffers de 256 KB; arquivos abertos com leitura sequencial.

### Estrutura

| Arquivo | Conteúdo |
|---|---|
| `src/lib.rs` | Biblioteca (lógica sem interface) |
| `src/folder.rs` | Comparação paralela das pastas |
| `src/filter.rs` | Filtro de extensões |
| `src/ops.rs` | Copiar, mover, apagar e renomear |
| `src/text.rs` | Leitura dos arquivos e detecção de codificação |
| `src/diff.rs` | Diff de linhas alinhado e diff dentro da linha |
| `src/main.rs` | Ponto de entrada e janela |
| `src/app.rs` | Estado da aplicação, painéis e threads de trabalho |
| `src/settings.rs` | Preferências e pastas recentes |
| `src/diff_view.rs` | Desenho do diff lado a lado |
| `src/tree_view.rs` | Árvore de arquivos virtualizada e menu do botão direito |
| `src/actions.rs` | Confirmação e execução das ações do menu |
| `src/editor.rs` | Janela de edição lado a lado |
| `examples/bench.rs` | Medição de desempenho |

### Próximos passos possíveis

- Rolagem horizontal para linhas longas (hoje elas são cortadas).
- Opções de comparação: ignorar espaços, maiúsculas e fim de linha.
- Filtros de pastas (ex.: ignorar `target/`).
- Realce de sintaxe com `syntect`.
- No editor: copiar um bloco de diferença de um lado para o outro com um clique.

[↑ Topo](#rustmerge)

---

<a id="en"></a>

## English

Folder and file comparison tool written in Rust, with an [egui](https://github.com/emilk/egui) interface.
The user interface is in Portuguese; the original labels are shown in quotes below.

### Features

- Compares two folders recursively, **in parallel** (multiple threads with `rayon`).
  - **"Conteúdo"** (content) method: size and, if equal, byte by byte.
  - **"Tamanho e data"** (size and date) method: does not open the files; much faster.
  - Progress bar and **"Cancelar"** (cancel) button.
  - **"Ignorar arquivos ocultos"** (ignore hidden files, on by default): skips files and folders
    with the Windows Hidden attribute and folders whose name starts with a dot (`.git`, `.vs`,
    `.idea`...), including everything inside them. Dot files such as `.gitignore` are still shown.
  - **"Extensões"** (extensions; field right below the folders, with a ⏷ menu of previously used
    filters): only compares files with the given extensions, separated by comma, semicolon or
    space (`rs, .toml *.txt tar.gz`). A leading `!` excludes (`!log`). Empty = all files. Folders
    left without any file are hidden. `Enter` in the field compares again.
- File tree on the right, colored by status (normal = identical, red = different,
  blue = left only, green = right only). The tree is virtualized: only visible rows are drawn,
  so it handles hundreds of thousands of files.
- Columns next to the name: **status**, **size** and **modification date** for each side.
  Column widths are adjusted by dragging the header dividers and are saved.
- Clicking a file shows a side-by-side diff with line numbers, filler lines aligning both sides
  and highlighting of changed characters. The file is read and compared in the background,
  without freezing the window.
- Navigation between differences: Previous/Next buttons or `Alt+↑` / `Alt+↓`. `F5` compares again.
- **"Editar"** (edit) in a file's right-click menu opens an editor window with both sides editable,
  line numbers and differing lines highlighted (updated as you type). Saves each side or both
  (`Ctrl+S`), keeping the file's encoding and line ending (CRLF/LF). If the file exists on only
  one side, saving the other side creates it. Closing with unsaved changes asks whether to save.
  When the window is closed, the file is compared again and the tree and diff are updated.
- **Right-click** on a file or folder: copy or move from left to right (or the other way),
  delete from one side or both (goes to the **Recycle Bin**) and rename (on both sides).
  Asks for confirmation before overwriting, moving or deleting.
- **Multiple selection** in the tree: `Ctrl+click` toggles, `Shift+click` selects a range.
  Right-click options apply to all selected items ("Editar" and "Renomear" only with one item).
  If a folder and something inside it are both selected, the operation applies to the folder.
  If an item fails, the others continue and the status bar shows a summary.
- **Find** (`Ctrl+F` or the "Localizar" button), in the main diff and in the editor window:
  searches one side at a time (**"Esquerda"** = left or **"Direita"** = right, chosen in the bar),
  highlights all matches, `Enter`/`F3` goes to the next one, `Shift+Enter`/`Shift+F3` to the
  previous one, optional case sensitivity, `Esc` closes. In the editor, the current match stays
  selected after closing.
- Remembers the last used folders and extension filters (⏷ menu next to each field), the method,
  the number of threads and the "ignore hidden files" and "show only differences" options.
  They are stored in `%APPDATA%\RustMerge\data\app.ron`.
- Detects UTF-8, UTF-16 (with BOM) and windows-1252; binary files are not displayed.

### Running

```
cargo run --release
cargo run --release -- C:\folder\left C:\folder\right
```

### Tests

```
cargo test
```

### Performance

The `bench` example generates test files and measures the comparison with 1, 2, 4... threads
(`gerar` = generate, `medir` = measure):

```
cargo run --release --example bench -- gerar C:\temp\bench 10000
cargo run --release --example bench -- medir C:\temp\bench\esquerda C:\temp\bench\direita
```

Measured on a 32-core machine with an SSD, 10,000 files per side (1.4 GB in total), warm cache:

| Threads | Time | Speedup |
|---|---|---|
| 1 | 0.61 s | 1.0x |
| 2 | 0.35 s | 1.7x |
| 4 | 0.18 s | 3.5x |
| 8 | 0.16 s | 3.9x |
| 16 | 0.21 s | 2.9x |
| 32 | 0.21 s | 2.9x |

Above 8 threads the disk and memory become the bottleneck, so automatic mode uses at most 8.
On a spinning hard drive, 1 or 2 threads is usually better.

Other optimizations:
- Files with different sizes are marked as different without being opened.
- Size and date come from the folder listing itself (no extra disk query on Windows).
- Each thread reuses 256 KB buffers; files are opened for sequential reading.

### Structure

| File | Contents |
|---|---|
| `src/lib.rs` | Library (logic without UI) |
| `src/folder.rs` | Parallel folder comparison |
| `src/filter.rs` | Extension filter |
| `src/ops.rs` | Copy, move, delete and rename |
| `src/text.rs` | File reading and encoding detection |
| `src/diff.rs` | Aligned line diff and in-line diff |
| `src/main.rs` | Entry point and window |
| `src/app.rs` | Application state, panels and worker threads |
| `src/settings.rs` | Preferences and recent folders |
| `src/diff_view.rs` | Side-by-side diff drawing |
| `src/tree_view.rs` | Virtualized file tree and right-click menu |
| `src/actions.rs` | Confirmation and execution of menu actions |
| `src/editor.rs` | Side-by-side editor window |
| `examples/bench.rs` | Performance measurement |

### Possible next steps

- Horizontal scrolling for long lines (currently they are cut off).
- Comparison options: ignore whitespace, case and line endings.
- Folder filters (e.g. ignore `target/`).
- Syntax highlighting with `syntect`.
- In the editor: copy a difference block from one side to the other with one click.

[↑ Top](#rustmerge)

---

<a id="es"></a>

## Español

Comparador de carpetas y archivos escrito en Rust, con interfaz [egui](https://github.com/emilk/egui).
La interfaz está en portugués; abajo se muestran entre comillas los nombres originales.

### Qué hace

- Compara dos carpetas de forma recursiva, **en paralelo** (varios hilos con `rayon`).
  - Método **"Conteúdo"** (contenido): tamaño y, si es igual, byte a byte.
  - Método **"Tamanho e data"** (tamaño y fecha): no abre los archivos; mucho más rápido.
  - Barra de progreso y botón **"Cancelar"**.
  - **"Ignorar arquivos ocultos"** (ignorar archivos ocultos, marcado por defecto): omite archivos
    y carpetas con el atributo Oculto de Windows y carpetas cuyo nombre empieza con punto (`.git`,
    `.vs`, `.idea`...), incluido todo su contenido. Los archivos con punto, como `.gitignore`,
    siguen apareciendo.
  - **"Extensões"** (extensiones; campo justo debajo de las carpetas, con menú ⏷ de los filtros
    ya usados): solo compara archivos con las extensiones indicadas, separadas por coma, punto y
    coma o espacio (`rs, .toml *.txt tar.gz`). Con `!` delante, excluye (`!log`). Vacío = todos.
    Las carpetas que quedan sin archivos se ocultan. `Enter` en el campo vuelve a comparar.
- Árbol de archivos a la derecha, coloreado por estado (normal = idéntico, rojo = diferente,
  azul = solo a la izquierda, verde = solo a la derecha). El árbol está virtualizado: solo dibuja
  las filas visibles, así que soporta cientos de miles de archivos.
- Columnas junto al nombre: **estado**, **tamaño** y **fecha de modificación** de cada lado.
  El ancho de las columnas se ajusta arrastrando los divisores del encabezado y se guarda.
- Al hacer clic en un archivo, muestra el diff lado a lado con números de línea, líneas de relleno
  que alinean ambos lados y resaltado de los caracteres cambiados. El archivo se lee y se compara
  en segundo plano, sin bloquear la ventana.
- Navegación entre diferencias: botones Anterior/Siguiente o `Alt+↑` / `Alt+↓`. `F5` vuelve a comparar.
- **"Editar"** en el menú del botón derecho de un archivo abre una ventana de edición con ambos
  lados editables, números de línea y las líneas diferentes resaltadas (se actualiza mientras
  escribes). Guarda cada lado o ambos (`Ctrl+S`), manteniendo la codificación y el salto de línea
  (CRLF/LF) del archivo. Si el archivo solo existe en un lado, guardar el otro lado lo crea. Al
  cerrar con cambios sin guardar, pregunta si quieres guardar. Al cerrar la ventana, el archivo
  se vuelve a comparar y se actualizan el árbol y el diff.
- **Botón derecho** en un archivo o carpeta: copiar o mover de izquierda a derecha (o al revés),
  borrar de un lado o de ambos (va a la **Papelera de reciclaje**) y renombrar (en ambos lados).
  Pide confirmación antes de reemplazar, mover o borrar.
- **Selección múltiple** en el árbol: `Ctrl+clic` marca/desmarca, `Shift+clic` selecciona un
  rango. Las opciones del botón derecho valen para todos los seleccionados ("Editar" y
  "Renomear" solo con un elemento). Si una carpeta y algo dentro de ella están seleccionados, la
  operación se aplica a la carpeta. Si algún elemento falla, los demás continúan y la barra de
  estado muestra el resumen.
- **Buscar** (`Ctrl+F` o el botón "Localizar"), en el diff de la pantalla principal y en la
  ventana de edición: busca en un lado a la vez (**"Esquerda"** = izquierda o **"Direita"** =
  derecha, elegido en la barra), resalta todas las coincidencias, `Enter`/`F3` va a la siguiente,
  `Shift+Enter`/`Shift+F3` a la anterior, opción de distinguir mayúsculas, `Esc` cierra. En el
  editor, al cerrar, la coincidencia actual queda seleccionada.
- Guarda las últimas carpetas y filtros de extensión usados (menú ⏷ junto a cada campo), el
  método, el número de hilos y las opciones "ignorar archivos ocultos" y "mostrar solo
  diferencias". Se guardan en `%APPDATA%\RustMerge\data\app.ron`.
- Detecta UTF-8, UTF-16 (con BOM) y windows-1252; los archivos binarios no se muestran.

### Cómo ejecutar

```
cargo run --release
cargo run --release -- C:\carpeta\izquierda C:\carpeta\derecha
```

### Pruebas

```
cargo test
```

### Rendimiento

El ejemplo `bench` genera archivos de prueba y mide la comparación con 1, 2, 4... hilos
(`gerar` = generar, `medir` = medir):

```
cargo run --release --example bench -- gerar C:\temp\bench 10000
cargo run --release --example bench -- medir C:\temp\bench\esquerda C:\temp\bench\direita
```

Medición en una máquina con 32 núcleos y SSD, 10.000 archivos por lado (1,4 GB en total), caché caliente:

| Hilos | Tiempo | Mejora |
|---|---|---|
| 1 | 0,61 s | 1,0x |
| 2 | 0,35 s | 1,7x |
| 4 | 0,18 s | 3,5x |
| 8 | 0,16 s | 3,9x |
| 16 | 0,21 s | 2,9x |
| 32 | 0,21 s | 2,9x |

Por encima de 8 hilos el disco y la memoria se convierten en el cuello de botella, por eso el
modo automático usa como máximo 8. En un disco duro mecánico, 1 o 2 hilos suele ser mejor.

Otras optimizaciones:
- Los archivos con tamaños diferentes se marcan como diferentes sin abrirlos.
- El tamaño y la fecha vienen del propio listado de la carpeta (sin consulta extra al disco en Windows).
- Cada hilo reutiliza búferes de 256 KB; los archivos se abren con lectura secuencial.

### Estructura

| Archivo | Contenido |
|---|---|
| `src/lib.rs` | Biblioteca (lógica sin interfaz) |
| `src/folder.rs` | Comparación paralela de carpetas |
| `src/filter.rs` | Filtro de extensiones |
| `src/ops.rs` | Copiar, mover, borrar y renombrar |
| `src/text.rs` | Lectura de archivos y detección de codificación |
| `src/diff.rs` | Diff de líneas alineado y diff dentro de la línea |
| `src/main.rs` | Punto de entrada y ventana |
| `src/app.rs` | Estado de la aplicación, paneles e hilos de trabajo |
| `src/settings.rs` | Preferencias y carpetas recientes |
| `src/diff_view.rs` | Dibujo del diff lado a lado |
| `src/tree_view.rs` | Árbol de archivos virtualizado y menú del botón derecho |
| `src/actions.rs` | Confirmación y ejecución de las acciones del menú |
| `src/editor.rs` | Ventana de edición lado a lado |
| `examples/bench.rs` | Medición de rendimiento |

### Próximos pasos posibles

- Desplazamiento horizontal para líneas largas (hoy se cortan).
- Opciones de comparación: ignorar espacios, mayúsculas y fin de línea.
- Filtros de carpetas (p. ej., ignorar `target/`).
- Resaltado de sintaxis con `syntect`.
- En el editor: copiar un bloque de diferencia de un lado al otro con un clic.

[↑ Inicio](#rustmerge)
