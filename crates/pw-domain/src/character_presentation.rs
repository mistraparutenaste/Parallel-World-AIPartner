#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterPresentationSettings {
    model_id: String,
    expression_id: String,
    motion_group: String,
    motion_index: u32,
    click_through: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterPresentationValidationError {
    UnknownModel,
    UnknownExpression,
    UnknownMotion,
}

impl CharacterPresentationSettings {
    pub fn try_new(
        model_id: &str,
        expression_id: &str,
        motion_group: &str,
        motion_index: u32,
        click_through: bool,
    ) -> Result<Self, CharacterPresentationValidationError> {
        let valid_expression = match model_id {
            "mark" => expression_id.is_empty(),
            "epsilon-free" => [
                "Angry",
                "Blushing",
                "f01",
                "f02",
                "Normal",
                "Sad",
                "Smile",
                "Surprised",
            ]
            .contains(&expression_id),
            _ => return Err(CharacterPresentationValidationError::UnknownModel),
        };
        if !valid_expression {
            return Err(CharacterPresentationValidationError::UnknownExpression);
        }
        let limit = match (model_id, motion_group) {
            ("mark", "Idle") => 6,
            ("epsilon-free", "Idle") => 1,
            ("epsilon-free", "FlickUp" | "Flick" | "Flick3" | "FlickDown" | "Shake") => 2,
            ("epsilon-free", "Tap") => 4,
            _ => return Err(CharacterPresentationValidationError::UnknownMotion),
        };
        if motion_index >= limit {
            return Err(CharacterPresentationValidationError::UnknownMotion);
        }
        Ok(Self {
            model_id: model_id.into(),
            expression_id: expression_id.into(),
            motion_group: motion_group.into(),
            motion_index,
            click_through,
        })
    }
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
    pub fn expression_id(&self) -> &str {
        &self.expression_id
    }
    pub fn motion_group(&self) -> &str {
        &self.motion_group
    }
    pub fn motion_index(&self) -> u32 {
        self.motion_index
    }
    pub fn click_through(&self) -> bool {
        self.click_through
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_phase_one_catalog_entries() {
        assert!(
            CharacterPresentationSettings::try_new("epsilon-free", "Smile", "Tap", 3, false)
                .is_ok()
        );
        assert!(CharacterPresentationSettings::try_new("mark", "", "Idle", 5, true).is_ok());
    }

    #[test]
    fn rejects_unknown_model_expression_and_motion() {
        assert_eq!(
            CharacterPresentationSettings::try_new("other", "", "Idle", 0, false),
            Err(CharacterPresentationValidationError::UnknownModel)
        );
        assert_eq!(
            CharacterPresentationSettings::try_new("epsilon-free", "Nope", "Idle", 0, false),
            Err(CharacterPresentationValidationError::UnknownExpression)
        );
        assert_eq!(
            CharacterPresentationSettings::try_new("epsilon-free", "Smile", "Nope", 0, false),
            Err(CharacterPresentationValidationError::UnknownMotion)
        );
        assert_eq!(
            CharacterPresentationSettings::try_new("epsilon-free", "Smile", "Tap", 4, false),
            Err(CharacterPresentationValidationError::UnknownMotion)
        );
    }
}
