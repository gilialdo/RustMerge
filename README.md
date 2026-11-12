# RustMerge

Comparador de pastas e arquivos em Rust, com interface [egui](https://github.com/emilk/egui).

## O que faz

- Compara duas pastas recursivamente, **em paralelo** (várias threads com `rayon`).
  - Método **Conteúdo**: tamanho e, se for igual, byte a byte.
  - Método **Tamanho e data**: não abre os arquivos; bem mais rápido.
  - Barra de progresso e botão **Cancelar**.
  - **Ignorar arquivos ocultos** (marcado por padrão): pula arquivos e pastas com o atributo
    Oculto do Windows e pastas cujo nome começa com ponto (`.git`, `.vs`, `.idea`...),
    incluindo tudo dentro delas. Arquivos com ponto, como `.gitignore`, continuam aparecendo.
  - **Extensões** (campo logo abaixo das pastas, com menu ⏷ dos filtros já usados): só compara arquivos com as extensões informadas, separadas por vírgula,
    ponto e vírgula ou espaço (`rs, .toml *.txt tar.gz`). Com `!` na frente, exclui (`!log`).
    Vazio = todos. Pastas que ficam sem nenhum arquivo são escondidas. `Enter` no campo compara de novo.
- Árvore de arquivos à direita, colorida por status (normal = idêntico, vermelho = diferente,
  azul = só na esquerda, verde = só na direita). A árvore é virtualizada: só desenha as linhas
  visíveis, então aguenta centenas de milhares de arquivos.
- Colunas ao lado do nome: **Situação**, **tamanho** e **data de modificação** de cada lado.
  A largura das colunas se ajusta arrastando as divisórias do cabeçalho e fica salva.
- Ao clicar num arquivo, mostra o diff lado a lado com números de linha, linhas de preenchimento
  alinhando os dois lados e destaque dos caracteres alterados. O arquivo é lido e comparado em
  segundo plano, sem travar a janela.
- Navegação entre diferenças: botões Anterior/Próxima ou `Alt+↑` / `Alt+↓`. `F5` compara de novo.
- **"Editar"** no menu do botão direito de um arquivo abre uma janela de edição com os dois lados editáveis, números de
  linha e as linhas diferentes destacadas (atualiza enquanto você digita). Salva cada lado ou os
  dois (`Ctrl+S`), mantendo a codificação e a quebra de linha (CRLF/LF) do arquivo. Se o arquivo
  só existe de um lado, salvar o outro lado cria o arquivo. Ao fechar com alterações não salvas,
  pergunta se quer salvar. Ao fechar a janela, o arquivo é recomparado e a árvore e o diff são
  atualizados.
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
- Guarda as últimas pastas e filtros de extensão usados (menu ⏷ ao lado de cada campo), o método, o número de threads
  e as opções "Ignorar arquivos ocultos" e "Mostrar só diferenças". Ficam em `%APPDATA%\RustMerge\data\app.ron`.
- Detecta UTF-8, UTF-16 (com BOM) e windows-1252; arquivos binários não são exibidos.

## Como rodar

```
cargo run --release
cargo run --release -- C:\pasta\esquerda C:\pasta\direita
```

## Testes

```
cargo test
```

## Desempenho

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

## Estrutura

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

## Próximos passos possíveis

- Rolagem horizontal para linhas longas (hoje elas são cortadas).
- Opções de comparação: ignorar espaços, maiúsculas e fim de linha.
- Filtros de arquivos (ex.: ignorar `target/`, `.git/`).
- Realce de sintaxe com `syntect`.
- No editor: copiar um bloco de diferença de um lado para o outro com um clique.
- Selecionar vários itens na árvore para copiar/mover/apagar de uma vez.
