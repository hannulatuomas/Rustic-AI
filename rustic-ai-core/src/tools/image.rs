use async_trait::async_trait;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct ImageTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageOperation {
    Resize,
    Crop,
    Rotate,
    Convert,
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputImageFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageOperation {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "resize" => Ok(Self::Resize),
            "crop" => Ok(Self::Crop),
            "rotate" => Ok(Self::Rotate),
            "convert" => Ok(Self::Convert),
            "metadata" => Ok(Self::Metadata),
            other => Err(Error::Tool(format!(
                "unsupported image operation '{other}' (expected resize|crop|rotate|convert|metadata)"
            ))),
        }
    }
}

impl OutputImageFormat {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(Self::Png),
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            "webp" => Ok(Self::Webp),
            "gif" => Ok(Self::Gif),
            other => Err(Error::Tool(format!(
                "unsupported image format '{other}' (expected png|jpeg|webp|gif)"
            ))),
        }
    }

    fn to_image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Webp => ImageFormat::WebP,
            Self::Gif => ImageFormat::Gif,
        }
    }
}

impl ImageTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {"type": "string", "enum": ["resize", "crop", "rotate", "convert", "metadata"]},
                "input": {"type": "string"},
                "output": {"type": "string"},
                "width": {"type": "integer", "minimum": 1},
                "height": {"type": "integer", "minimum": 1},
                "x": {"type": "integer", "minimum": 0},
                "y": {"type": "integer", "minimum": 0},
                "degrees": {"type": "integer", "enum": [90, 180, 270]},
                "format": {"type": "string", "enum": ["png", "jpeg", "webp", "gif"]},
                "filter": {"type": "string", "enum": ["nearest", "triangle", "catmullrom", "gaussian", "lanczos3"]}
            },
            "required": ["operation", "input"]
        });
        Self { config, schema }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn optional_string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    fn canonicalize(path: &Path) -> Result<PathBuf> {
        std::fs::canonicalize(path).map_err(|err| {
            Error::Tool(format!(
                "failed to resolve path '{}': {err}",
                path.display()
            ))
        })
    }

    fn resolve_input_path(&self, context: &ToolExecutionContext, raw: &str) -> Result<PathBuf> {
        let base = Self::canonicalize(&context.working_directory)?;
        let candidate = {
            let p = PathBuf::from(raw);
            if p.is_absolute() {
                p
            } else {
                base.join(p)
            }
        };
        let resolved = Self::canonicalize(&candidate)?;
        if !resolved.starts_with(&base) {
            return Err(Error::Tool(format!(
                "input '{}' is outside tool working directory '{}'; use a path within the workspace",
                resolved.display(),
                base.display()
            )));
        }
        Ok(resolved)
    }

    fn resolve_output_path(&self, context: &ToolExecutionContext, raw: &str) -> Result<PathBuf> {
        let base = Self::canonicalize(&context.working_directory)?;
        let candidate = {
            let p = PathBuf::from(raw);
            if p.is_absolute() {
                p
            } else {
                base.join(p)
            }
        };
        let parent = candidate.parent().ok_or_else(|| {
            Error::Tool(format!(
                "output path '{}' has no parent",
                candidate.display()
            ))
        })?;
        if !parent.exists() {
            return Err(Error::Tool(format!(
                "output parent '{}' does not exist",
                parent.display()
            )));
        }
        let parent_resolved = Self::canonicalize(parent)?;
        if !parent_resolved.starts_with(&base) {
            return Err(Error::Tool(format!(
                "output '{}' is outside tool working directory '{}'; use a path within the workspace",
                candidate.display(),
                base.display()
            )));
        }
        Ok(candidate)
    }

    fn filter_from_args(args: &Value) -> FilterType {
        match args
            .get("filter")
            .and_then(Value::as_str)
            .unwrap_or("lanczos3")
            .to_ascii_lowercase()
            .as_str()
        {
            "nearest" => FilterType::Nearest,
            "triangle" => FilterType::Triangle,
            "catmullrom" => FilterType::CatmullRom,
            "gaussian" => FilterType::Gaussian,
            _ => FilterType::Lanczos3,
        }
    }

    fn save_image(
        image: &DynamicImage,
        output: &Path,
        format: Option<OutputImageFormat>,
    ) -> Result<()> {
        if let Some(format) = format {
            image
                .save_with_format(output, format.to_image_format())
                .map_err(|err| Error::Tool(format!("failed to save '{}': {err}", output.display())))
        } else {
            image
                .save(output)
                .map_err(|err| Error::Tool(format!("failed to save '{}': {err}", output.display())))
        }
    }

    fn run_operation(
        &self,
        args: &Value,
        context: &ToolExecutionContext,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<Value> {
        let operation = ImageOperation::parse(Self::required_string(args, "operation")?)?;
        let input_path = self.resolve_input_path(context, Self::required_string(args, "input")?)?;

        if let Some(tx) = tx.as_ref() {
            let _ = tx.try_send(Event::ToolOutput {
                tool: self.config.name.clone(),
                stdout_chunk: format!("loading image '{}'\n", input_path.display()),
                stderr_chunk: String::new(),
            });
        }

        let mut image = image::open(&input_path).map_err(|err| {
            Error::Tool(format!(
                "failed to open image '{}': {err}",
                input_path.display()
            ))
        })?;
        let (input_width, input_height) = image.dimensions();

        let payload = match operation {
            ImageOperation::Metadata => {
                let bytes = std::fs::read(&input_path).map_err(|err| {
                    Error::Tool(format!("failed to read '{}': {err}", input_path.display()))
                })?;
                let guessed = image::guess_format(&bytes).ok();
                json!({
                    "operation": "metadata",
                    "input": input_path,
                    "width": input_width,
                    "height": input_height,
                    "color": format!("{:?}", image.color()),
                    "guessed_format": guessed.map(|v| format!("{:?}", v)),
                })
            }
            ImageOperation::Resize => {
                let width = args
                    .get("width")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Error::Tool("missing 'width' for resize".to_owned()))?
                    as u32;
                let height = args
                    .get("height")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Error::Tool("missing 'height' for resize".to_owned()))?
                    as u32;
                let filter = Self::filter_from_args(args);
                image = image.resize_exact(width, height, filter);

                let output =
                    self.resolve_output_path(context, Self::required_string(args, "output")?)?;
                let format = Self::optional_string(args, "format")
                    .map(OutputImageFormat::parse)
                    .transpose()?;
                Self::save_image(&image, &output, format)?;

                json!({
                    "operation": "resize",
                    "input": input_path,
                    "output": output,
                    "from": { "width": input_width, "height": input_height },
                    "to": { "width": width, "height": height },
                })
            }
            ImageOperation::Crop => {
                let x = args.get("x").and_then(Value::as_u64).unwrap_or(0) as u32;
                let y = args.get("y").and_then(Value::as_u64).unwrap_or(0) as u32;
                let width = args
                    .get("width")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Error::Tool("missing 'width' for crop".to_owned()))?
                    as u32;
                let height = args
                    .get("height")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Error::Tool("missing 'height' for crop".to_owned()))?
                    as u32;
                if x >= input_width || y >= input_height {
                    return Err(Error::Tool(
                        "crop origin is outside image bounds".to_owned(),
                    ));
                }
                let crop_w = width.min(input_width - x);
                let crop_h = height.min(input_height - y);
                image = image.crop_imm(x, y, crop_w, crop_h);

                let output =
                    self.resolve_output_path(context, Self::required_string(args, "output")?)?;
                let format = Self::optional_string(args, "format")
                    .map(OutputImageFormat::parse)
                    .transpose()?;
                Self::save_image(&image, &output, format)?;

                json!({
                    "operation": "crop",
                    "input": input_path,
                    "output": output,
                    "crop": { "x": x, "y": y, "width": crop_w, "height": crop_h },
                })
            }
            ImageOperation::Rotate => {
                let degrees = args.get("degrees").and_then(Value::as_u64).unwrap_or(90) as u32;
                image = match degrees {
                    90 => image.rotate90(),
                    180 => image.rotate180(),
                    270 => image.rotate270(),
                    _ => {
                        return Err(Error::Tool("rotate supports degrees=90|180|270".to_owned()));
                    }
                };

                let output =
                    self.resolve_output_path(context, Self::required_string(args, "output")?)?;
                let format = Self::optional_string(args, "format")
                    .map(OutputImageFormat::parse)
                    .transpose()?;
                Self::save_image(&image, &output, format)?;

                json!({
                    "operation": "rotate",
                    "input": input_path,
                    "output": output,
                    "degrees": degrees,
                })
            }
            ImageOperation::Convert => {
                let output =
                    self.resolve_output_path(context, Self::required_string(args, "output")?)?;
                let format = OutputImageFormat::parse(Self::required_string(args, "format")?)?;
                Self::save_image(&image, &output, Some(format))?;

                json!({
                    "operation": "convert",
                    "input": input_path,
                    "output": output,
                    "format": format!("{:?}", format.to_image_format()),
                })
            }
        };

        if let Some(tx) = tx {
            let _ = tx.try_send(Event::ToolOutput {
                tool: self.config.name.clone(),
                stdout_chunk: "image operation completed\n".to_owned(),
                stderr_chunk: String::new(),
            });
        }

        Ok(payload)
    }

    async fn execute_operation(
        &self,
        args: Value,
        context: &ToolExecutionContext,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<ToolResult> {
        let payload = self.run_operation(&args, context, tx)?;
        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
    }
}

#[async_trait]
impl Tool for ImageTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Image processing operations (resize/crop/rotate/convert/metadata)"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        self.execute_operation(args, context, None).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = self.name().to_owned();
        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let result = self
            .execute_operation(args, context, Some(tx.clone()))
            .await;
        match &result {
            Ok(tool_result) => {
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("image tool failed: {err}")));
            }
        }

        result
    }
}
